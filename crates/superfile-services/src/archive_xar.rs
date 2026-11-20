//! XAR stores a zlib-compressed XML catalog followed by independently encoded
//! file ranges. Read the catalog under the structured-document working budget.
use super::*;
use std::io::{Seek, SeekFrom};

fn field<'a>(node: roxmltree::Node<'a, '_>, key: &str) -> Result<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(key))
        .and_then(|n| n.text())
        .with_context(|| format!("XAR field {key} is missing"))
}

pub(super) fn read(mut input: Reader<File>, entries: &mut Members<'_>) -> Result<()> {
    let length = input.inner.metadata()?.len();
    let mut header = [0; 28];
    input.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"xar!" && header[6..8] == [0, 1],
        "Invalid XAR header"
    );
    let offset = u64::from(u16::from_be_bytes(header[4..6].try_into()?));
    let compressed = u64::from_be_bytes(header[8..16].try_into()?);
    let expanded = u64::from_be_bytes(header[16..24].try_into()?);
    let heap = offset
        .checked_add(compressed)
        .context("Invalid XAR catalog length")?;
    ensure!(
        offset >= 28 && heap <= length && expanded <= crate::limits::ARCHIVE_BYTES,
        "Invalid or oversized XAR catalog"
    );
    input.seek(SeekFrom::Start(offset))?;
    let mut xml = String::new();
    flate2::read::ZlibDecoder::new(input.by_ref().take(compressed))
        .take(expanded + 1)
        .read_to_string(&mut xml)?;
    ensure!(xml.len() as u64 == expanded, "XAR catalog length mismatch");
    let doc = roxmltree::Document::parse(&xml)?;
    let toc = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("toc"))
        .context("Missing XAR catalog")?;
    let mut stack = vec![(toc, String::new())];
    while let Some((parent, prefix)) = stack.pop() {
        for node in parent.children().filter(|n| n.has_tag_name("file")) {
            entries.cancel.check()?;
            let name = field(node, "name")?;
            ensure!(
                !name.contains('/') && name != "." && name != "..",
                "Invalid XAR member name"
            );
            let full = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let kind = field(node, "type")?;
            if kind == "directory" {
                entries.record(&full, 0, true)?;
                stack.push((node, full));
                continue;
            }
            ensure!(kind == "file", "Unsupported XAR member type {kind}");
            let Some(data) = node.children().find(|n| n.has_tag_name("data")) else {
                entries.record(&full, 0, false)?;
                continue;
            };
            let size = field(data, "size")?.parse::<u64>()?;
            let packed = field(data, "length")?.parse::<u64>()?;
            let start = heap
                .checked_add(field(data, "offset")?.parse::<u64>()?)
                .context("Invalid XAR offset")?;
            ensure!(
                start <= length && packed <= length - start,
                "Truncated XAR member"
            );
            if let Some(file) = entries.record(&full, size, false)? {
                input.seek(SeekFrom::Start(start))?;
                let range = input.by_ref().take(packed);
                let encoding = data
                    .children()
                    .find(|n| n.has_tag_name("encoding"))
                    .and_then(|n| n.attribute("style"))
                    .unwrap_or("application/octet-stream");
                let mut decoded: Box<dyn Read + '_> = match encoding {
                    "application/octet-stream" => Box::new(range),
                    "application/x-gzip" | "application/zlib" => {
                        Box::new(flate2::read::ZlibDecoder::new(range))
                    }
                    "application/x-bzip2" => Box::new(bzip2::read::BzDecoder::new(range)),
                    "application/x-lzma" | "application/x-xz" => {
                        let stream = xz2::stream::Stream::new_auto_decoder(
                            u64::from(DECODER_KIB) * 1024,
                            0,
                        )?;
                        Box::new(xz2::read::XzDecoder::new_stream(range, stream))
                    }
                    _ => anyhow::bail!("Unsupported XAR encoding {encoding}"),
                };
                entries.copy(&mut *decoded, file, size)?;
            }
        }
    }
    Ok(())
}
