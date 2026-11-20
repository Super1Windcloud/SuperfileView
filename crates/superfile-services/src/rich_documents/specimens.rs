//! Draw outlines from the opened face itself. Vector ink can follow the
//! document palette without installing untrusted fonts in the OS or GPUI.
use crate::{snapshot::MappedSnapshot, work::Cancellation};
use anyhow::{Result, ensure};
use pdf_writer::{Content, Pdf, Rect, Ref};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
    raw::FileRef,
    string::StringId,
};
use std::path::Path;

struct Pen<'a> {
    content: &'a mut Content,
    last: (f32, f32),
}
impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.content.move_to(x, y);
        self.last = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.content.line_to(x, y);
        self.last = (x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (sx, sy) = self.last;
        self.curve_to(
            (sx + 2. * cx) / 3.,
            (sy + 2. * cy) / 3.,
            (x + 2. * cx) / 3.,
            (y + 2. * cy) / 3.,
            x,
            y,
        );
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.content.cubic_to(x1, y1, x2, y2, x, y);
        self.last = (x, y);
    }
    fn close(&mut self) {
        self.content.close_path();
    }
}

fn line(font: &FontRef<'_>, stream: &mut Content, text: &str, y: f32, size: f32) -> Result<()> {
    let outlines = font.outline_glyphs();
    let map = font.charmap();
    let size = Size::new(size);
    let location = LocationRef::default();
    let metrics = font.glyph_metrics(size, location);
    let mut x = 28.;
    for ch in text.chars() {
        if x > 560. {
            break;
        }
        let Some(id) = map.map(ch) else { continue };
        if let Some(outline) = outlines.get(id) {
            stream.save_state().transform([1., 0., 0., 1., x, y]);
            let mut pen = Pen {
                content: stream,
                last: (0., 0.),
            };
            outline.draw(DrawSettings::unhinted(size, location), &mut pen)?;
            stream.fill_nonzero().restore_state();
        }
        x += metrics.advance_width(id).unwrap_or_default();
    }
    Ok(())
}

pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<tempfile::TempPath> {
    let snapshot = MappedSnapshot::open(path, cancel)?;
    let required = (snapshot.as_ref().len() as u64)
        .saturating_mul(2)
        .max(1024 * 1024);
    let _memory = crate::resources::pool().reserve_working(required, required)?;
    let mut pdf = Pdf::new();
    let tree = Ref::new(2);
    pdf.catalog(Ref::new(1)).pages(tree);
    let mut pages = Vec::new();
    let mut append = |bytes: &[u8]| -> Result<()> {
        for font in FileRef::new(bytes)?.fonts() {
            cancel.check()?;
            let font = font?;
            let page = Ref::new(3 + i32::try_from(pages.len())? * 2);
            let content_id = Ref::new(page.get() + 1);
            let mut stream = Content::new();
            let name = font
                .localized_strings(StringId::FULL_NAME)
                .english_or_first()
                .map(|v| v.to_string())
                .unwrap_or_default();
            line(&font, &mut stream, &name, 286., 25.)?;
            for (text, y) in [
                ("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 235.),
                ("abcdefghijklmnopqrstuvwxyz", 190.),
                ("0123456789 !?#$%&()", 145.),
                ("中文字体示例 日本語 한글 Ελληνικά", 100.),
            ] {
                line(&font, &mut stream, text, y, 24.)?;
            }
            // Symbol fonts may have no Latin or CJK mappings.
            let mapped = font
                .charmap()
                .mappings()
                .filter_map(|(cp, _)| char::from_u32(cp))
                .filter(|ch| !ch.is_control() && !ch.is_whitespace())
                .take(26)
                .collect::<String>();
            line(&font, &mut stream, &mapped, 47., 22.)?;
            ensure!(
                !stream.as_bytes().is_empty(),
                "Font contains no displayable outline glyphs"
            );
            pdf.page(page)
                .parent(tree)
                .media_box(Rect::new(0., 0., 600., 328.))
                .contents(content_id);
            crate::limits::preview_budget(
                pdf.as_bytes().len().saturating_add(stream.len()) as u64 <= required,
                "Font specimen exceeds preview memory",
            )?;
            pdf.stream(content_id, &stream.finish());
            pages.push(page);
        }
        Ok(())
    };
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dfont"))
    {
        let fork = macbinary::ResourceFork::new(snapshot.as_ref())
            .map_err(|e| anyhow::anyhow!("Invalid font resource map: {e:?}"))?;
        for kind in fork
            .resource_types()
            .filter(|t| t.resource_type().0 == u32::from_be_bytes(*b"sfnt"))
        {
            for resource in fork.resources(kind) {
                append(resource.data())?;
            }
        }
    } else {
        append(snapshot.as_ref())?;
    }
    ensure!(!pages.is_empty(), "Font contains no OpenType faces");
    pdf.pages(tree)
        .kids(pages.iter().copied())
        .count(i32::try_from(pages.len())?);
    super::fixed_layout::temp_pdf(&pdf.finish(), cancel)
}
