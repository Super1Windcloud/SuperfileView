//! On-demand PDF rasterization with a pure Rust renderer and an immutable disk snapshot.
use crate::{snapshot::MappedSnapshot, work::Cancellation};
use anyhow::{Result, ensure};
use std::{path::Path, sync::Arc};

pub struct PdfDocument {
    pdf: hayro::hayro_syntax::Pdf,
}
pub struct PdfPage {
    pub png: Vec<u8>,
    pub pages: usize,
}
impl PdfDocument {
    pub fn open(path: &Path, cancel: &Cancellation) -> Result<Self> {
        let snapshot = Arc::new(MappedSnapshot::open(path, cancel)?);
        let pdf = hayro::hayro_syntax::Pdf::new(snapshot)
            .map_err(|error| anyhow::anyhow!("Cannot read PDF: {error:?}"))?;
        ensure!(!pdf.pages().is_empty(), "PDF has no pages");
        cancel.check()?;
        Ok(Self { pdf })
    }
    pub fn render(&self, index: usize, cancel: &Cancellation) -> Result<PdfPage> {
        cancel.check()?;
        let page = self
            .pdf
            .pages()
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("PDF page is out of range"))?;
        let (width, height) = page.render_dimensions();
        ensure!(
            width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
            "Invalid PDF page dimensions"
        );
        let scale = (crate::limits::IMAGE_PREVIEW_EDGE as f32 / width.max(height)).min(2.0);
        let pixels = ((width * scale).ceil() as u64)
            .checked_mul((height * scale).ceil() as u64)
            .ok_or_else(|| anyhow::anyhow!("PDF dimensions overflow"))?;
        let required = pixels
            .checked_mul(16)
            .ok_or_else(|| anyhow::anyhow!("PDF render budget overflow"))?;
        let _memory = crate::resources::pool().reserve_working(required, required)?;
        let pixmap = hayro::render(
            page,
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        cancel.check()?;
        let png = pixmap
            .into_png()
            .map_err(|error| anyhow::anyhow!("Cannot encode PDF page: {error}"))?;
        Ok(PdfPage {
            png,
            pages: self.pdf.pages().len(),
        })
    }
}
