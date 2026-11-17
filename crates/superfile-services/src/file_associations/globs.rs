//! File-name suffixes from the shared MIME database. Patterns without a plain
//! suffix remain searchable by their MIME identifier rather than an invented extension.
use crate::work::Cancellation;
use anyhow::Result;
use std::collections::BTreeMap;

pub(super) fn parse(
    text: &str,
    result: &mut BTreeMap<String, Vec<String>>,
    cancel: &Cancellation,
) -> Result<()> {
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        cancel.check()?;
        let mut fields = line.split(':');
        let (_weight, Some(mime), Some(pattern)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if let Some(extension) = pattern.strip_prefix("*.")
            && !extension.is_empty()
            && !extension.contains(['*', '?', '[', ']', '/', '\\'])
        {
            let extensions = result.entry(mime.to_owned()).or_default();
            if !extensions.iter().any(|value| value == extension) {
                extensions.push(extension.to_owned());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_preserve_case_and_compound_formats_without_treating_globs_as_extensions() {
        let mut formats = BTreeMap::new();
        parse("# comment\n50:application/pdf:*.pdf\n60:application/pdf:*.pdf\n50:application/gzip:*.tar.gz\n50:text/x-c++src:*.C:cs\n50:text/plain:README\n50:text/x-test:*.foo[0-9]\nmalformed", &mut formats, &Cancellation::default()).unwrap();
        assert_eq!(formats["application/pdf"], ["pdf"]);
        assert_eq!(formats["application/gzip"], ["tar.gz"]);
        assert_eq!(formats["text/x-c++src"], ["C"]);
        assert_eq!(formats.len(), 3);
        let (cancel, guard) = Cancellation::task();
        drop(guard);
        assert!(parse("50:application/pdf:*.pdf", &mut formats, &cancel).is_err());
    }
}
