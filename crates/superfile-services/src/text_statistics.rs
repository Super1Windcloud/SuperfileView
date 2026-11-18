//! Source-text counts for complete buffers and bounded, editable file pages.
use crate::{paged::TextPage, work::Cancellation};
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStatistics {
    pub words: u64,
    pub lines: u64,
}

#[derive(Clone, Copy, Default)]
struct Summary {
    words: u64,
    breaks: u64,
    first: Option<char>,
    last: Option<char>,
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3005 | 0x3007 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0x20000..=0x2FA1F | 0x30000..=0x323AF)
}

fn is_word_character(ch: char) -> bool {
    ch.is_alphanumeric() && !is_han(ch)
}

impl Summary {
    fn push(&mut self, text: &str) {
        for ch in text.chars() {
            self.first.get_or_insert(ch);
            if is_han(ch) || (is_word_character(ch) && !self.last.is_some_and(is_word_character)) {
                self.words += 1;
            }
            if ch == '\r' || (ch == '\n' && self.last != Some('\r')) {
                self.breaks += 1;
            }
            self.last = Some(ch);
        }
    }

    fn append(&mut self, right: Self) {
        let Some(first) = right.first else { return };
        self.words += right.words
            - u64::from(self.last.is_some_and(is_word_character) && is_word_character(first));
        self.breaks += right.breaks - u64::from(self.last == Some('\r') && first == '\n');
        self.first.get_or_insert(first);
        self.last = right.last;
    }

    fn finish(self) -> TextStatistics {
        TextStatistics {
            words: self.words,
            lines: self.breaks + u64::from(self.last.is_some_and(|ch| ch != '\r' && ch != '\n')),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct SurroundingText {
    before: Summary,
    after: Summary,
}

impl SurroundingText {
    pub fn count<'a>(
        self,
        chunks: impl IntoIterator<Item = &'a str>,
        cancel: &Cancellation,
    ) -> Result<TextStatistics> {
        let mut summary = self.before;
        for chunk in chunks {
            cancel.check()?;
            summary.push(chunk);
        }
        summary.append(self.after);
        Ok(summary.finish())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TextRegion {
    path: PathBuf,
    total: u64,
    modified: Option<SystemTime>,
    identity: (u64, u64),
    start: u64,
    end: u64,
}

impl TextRegion {
    pub fn from_page(page: &TextPage) -> Option<Self> {
        if !page.source.is_editable() {
            return None;
        }
        Some(Self {
            path: page.source.original.clone(),
            total: page.total,
            modified: page.modified,
            identity: page.identity,
            start: page.start,
            end: page.end,
        })
    }

    pub fn same_file(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.total == other.total
            && self.modified == other.modified
            && self.path == other.path
    }

    fn verify(&self, file: &File) -> Result<()> {
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.len() == self.total
                && metadata.modified().ok() == self.modified
                && crate::platform::file_identity(file)? == self.identity,
            "File changed while counting text"
        );
        Ok(())
    }

    pub fn count_surrounding(&self, cancel: &Cancellation) -> Result<SurroundingText> {
        cancel.check()?;
        ensure!(
            self.start <= self.end && self.end <= self.total,
            "Invalid text range"
        );
        let mut file = File::open(&self.path)?;
        self.verify(&file)?;
        let before = read_summary((&mut file).take(self.start), cancel)?;
        file.seek(SeekFrom::Start(self.end))?;
        let after = read_summary((&mut file).take(self.total - self.end), cancel)?;
        self.verify(&File::open(&self.path)?)?;
        Ok(SurroundingText { before, after })
    }
}

fn read_summary(mut reader: impl Read, cancel: &Cancellation) -> Result<Summary> {
    let mut summary = Summary::default();
    let mut buffer = vec![0; crate::limits::IO_BUFFER_BYTES];
    let mut carry = 0;
    loop {
        cancel.check()?;
        let read = match reader.read(&mut buffer[carry..]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            ensure!(carry == 0, "Incomplete UTF-8 in text statistics");
            return Ok(summary);
        }
        let length = carry + read;
        let bytes = &buffer[..length];
        ensure!(
            !bytes.contains(&0),
            "Binary content cannot be counted as UTF-8 text"
        );
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                summary.push(text);
                carry = 0;
            }
            Err(error) if error.error_len().is_none() => {
                let split = error.valid_up_to();
                summary.push(std::str::from_utf8(&bytes[..split])?);
                buffer.copy_within(split..length, 0);
                carry = length - split;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_and_physical_lines_across_text_chunks() {
        let cancel = Cancellation::default();
        let scan = SurroundingText::default();
        assert_eq!(
            scan.count([""], &cancel).unwrap(),
            TextStatistics::default()
        );
        assert_eq!(
            scan.count(["# Super", "file 中文\r", "\n\n𠀀 42\rEnd\n"], &cancel)
                .unwrap(),
            TextStatistics { words: 6, lines: 4 }
        );
        assert_eq!(
            scan.count(["a very long line without a final newline"], &cancel)
                .unwrap()
                .lines,
            1
        );
    }

    #[test]
    fn counts_the_whole_file_with_the_edited_page_and_split_utf8() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("words.txt");
        let prefix = format!("{}中 hello", "a".repeat(crate::limits::IO_BUFFER_BYTES - 1));
        let old_page = " old text\r";
        let after = "\nworld\n";
        std::fs::write(&path, format!("{prefix}{old_page}{after}"))?;
        let page = crate::paged::read_page(&path, 0, false)?;
        let mut region = TextRegion::from_page(&page).unwrap();
        region.start = prefix.len() as u64;
        region.end = (prefix.len() + old_page.len()) as u64;
        let cancel = Cancellation::default();
        let surrounding = region.count_surrounding(&cancel)?;
        assert_eq!(
            surrounding.count([old_page], &cancel)?,
            TextStatistics { words: 6, lines: 2 }
        );
        assert_eq!(
            surrounding.count(["there\r"], &cancel)?,
            TextStatistics { words: 4, lines: 2 }
        );
        std::fs::write(path, "different file")?;
        assert!(region.count_surrounding(&cancel).is_err());
        Ok(())
    }
}
