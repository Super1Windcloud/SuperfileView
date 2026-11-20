//! PDFCraft page rendering over an immutable document snapshot.
use crate::{resources::WorkingReservation, snapshot::MappedSnapshot, work::Cancellation};
use anyhow::{Result, bail, ensure};
use hayro::hayro_syntax::Pdf;
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, device_pixels};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub struct PdfDocument {
    sizes: Vec<(f32, f32)>,
    backend: Mutex<Backend>,
}
enum Backend {
    Buffered {
        renderer: Box<PageRenderer>,
        _input: WorkingReservation,
    },
    // PageRenderer requires an owned Vec. Keep large inputs on disk with the
    // same patched rasterizer shipped by PDFCraft when that Vec cannot fit.
    Mapped(Pdf),
}
pub struct PdfPage {
    pub png: Vec<u8>,
    pub pages: usize,
}
impl PdfDocument {
    pub fn open(path: &Path, cancel: &Cancellation) -> Result<Self> {
        let snapshot = Arc::new(MappedSnapshot::open(path, cancel)?);
        let bytes: &[u8] = snapshot.as_ref().as_ref();
        let pdf = Pdf::new(snapshot.clone())
            .map_err(|error| anyhow::anyhow!("Cannot read PDF: {error:?}"))?;
        let mut sizes = Vec::new();
        for page in pdf.pages().iter() {
            cancel.check()?;
            sizes.push(page.render_dimensions());
        }
        ensure!(!sizes.is_empty(), "PDF has no pages");
        let length = bytes.len() as u64;
        // Leave room for at least one preview while the input is resident.
        let headroom = u64::from(crate::limits::IMAGE_PREVIEW_EDGE).pow(2) * 16;
        let backend = if let Ok(mut input) = crate::resources::pool().reserve_working(
            length.saturating_add(headroom),
            length.saturating_add(headroom),
        ) {
            drop(pdf);
            let mut owned = Vec::with_capacity(bytes.len());
            cancel.copy(bytes, &mut owned)?;
            input.shrink_to(length);
            let renderer = PageRenderer::new(Arc::new(owned), RenderConfig::default().on_screen());
            ensure!(
                renderer.page_count() == sizes.len(),
                "Cannot open PDF renderer"
            );
            Backend::Buffered {
                renderer: Box::new(renderer),
                _input: input,
            }
        } else {
            Backend::Mapped(pdf)
        };
        cancel.check()?;
        Ok(Self {
            sizes,
            backend: Mutex::new(backend),
        })
    }
    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    pub fn page_ratio(&self, index: usize) -> Option<f32> {
        let (w, h) = *self.sizes.get(index)?;
        let ratio = w / h;
        (ratio.is_finite() && ratio > 0.).then_some(ratio)
    }

    pub fn render(&self, index: usize, cancel: &Cancellation) -> Result<PdfPage> {
        cancel.check()?;
        let &(width, height) = self
            .sizes
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("PDF page is out of range"))?;
        ensure!(
            width.is_finite() && height.is_finite() && width > 0. && height > 0.,
            "Invalid PDF page dimensions"
        );
        let scale = (crate::limits::IMAGE_PREVIEW_EDGE as f32 / width.max(height)).min(2.0);
        let w = device_pixels(width, scale);
        let h = device_pixels(height, scale);
        let required = u64::from(w) * u64::from(h) * 16;
        let mut backend = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        cancel.check()?;
        let _memory = crate::resources::pool().reserve_working(required, required)?;
        let png = match &mut *backend {
            Backend::Buffered { renderer, .. } => {
                let result = renderer.render(RenderRequest {
                    page: index,
                    scale,
                    ..Default::default()
                });
                cancel.check()?;
                if let Some(message) = result.error {
                    bail!("{message}");
                }
                let mut png = Vec::new();
                PngEncoder::new(&mut png).write_image(
                    &result.rgba,
                    result.width,
                    result.height,
                    ExtendedColorType::Rgba8,
                )?;
                png
            }
            Backend::Mapped(pdf) => {
                let pixmap = hayro::render(
                    &pdf.pages()[index],
                    &hayro::RenderCache::new(),
                    &hayro::hayro_interpret::InterpreterSettings::default(),
                    &hayro::RenderSettings {
                        x_scale: scale,
                        y_scale: scale,
                        width: Some(w as u16),
                        height: Some(h as u16),
                        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                        ..Default::default()
                    },
                );
                cancel.check()?;
                pixmap.into_png()?
            }
        };
        Ok(PdfPage {
            png,
            pages: self.page_count(),
        })
    }
}
