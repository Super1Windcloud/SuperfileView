//! Decode Apple binary property lists to private, read-only XML preview pages.
use crate::{paged::TextPage, work::Cancellation};
use anyhow::Result;
use plist_parser::stream::{BinaryReader, Event, Writer, XmlWriter};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

pub fn is_binary(path: &Path) -> bool {
    if !superfile_core::formats::FormatSet::BinaryPlist.matches(path) {
        return false;
    }
    let mut header = [0; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header == b"bplist00"
}
pub fn read(path: &Path, cancel: &Cancellation) -> Result<TextPage> {
    cancel.check()?;
    let reader = BinaryReader::new(BufReader::new(File::open(path)?));
    let mut output = tempfile::NamedTempFile::new()?;
    let mut buffer = BufWriter::new(output.as_file_mut());
    let mut xml = XmlWriter::new(&mut buffer);
    for event in reader {
        cancel.check()?;
        match event? {
            // XML plists represent keyed-archive UIDs as CF$UID dictionaries.
            Event::Uid(uid) => {
                xml.write(Event::StartDictionary(Some(1)))?;
                xml.write(Event::String("CF$UID".into()))?;
                xml.write(Event::Integer(uid.get().into()))?;
                xml.write(Event::EndCollection)?;
            }
            value => xml.write(value)?,
        }
    }
    buffer.flush()?;
    drop(buffer);
    cancel.check()?;
    crate::paged::generated(path, output)
}
