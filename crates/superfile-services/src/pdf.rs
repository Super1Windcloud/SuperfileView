//! PDFCraft page rendering over an immutable document snapshot.
use crate::{
    resources::{ResourcePool, WorkingReservation, pool},
    snapshot::MappedSnapshot,
    work::Cancellation,
};
use anyhow::{Result, bail, ensure};
use hayro::hayro_syntax::Pdf;
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use pdfcraft_render::{
    PageRenderer, ReadingColors, ReadingParams, RenderConfig, RenderRequest, device_pixels,
    effective_scale,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use superfile_core::reader_palette::ReaderPalette;

pub struct PdfDocument {
    sizes: Vec<(f32, f32)>,
    backend: Mutex<Backend>,
}
enum Backend {
    Buffered {
        renderer: Box<PageRenderer>,
        bytes: Arc<Vec<u8>>,
        colors: ReadingColors,
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
            let bytes = Arc::new(owned);
            let renderer = PageRenderer::new(bytes.clone(), RenderConfig::default().on_screen());
            ensure!(
                renderer.page_count() == sizes.len(),
                "Cannot open PDF renderer"
            );
            Backend::Buffered {
                renderer: Box::new(renderer),
                colors: ReadingColors::default(),
                bytes,
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
        self.render_with_palette(index, None, cancel)
    }

    pub fn render_with_palette(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        cancel: &Cancellation,
    ) -> Result<PdfPage> {
        self.render_page(index, palette, None, cancel)
    }

    /// Render the entire page for a viewport whose longer edge is in device pixels.
    pub fn render_at_edge(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        edge: u32,
        cancel: &Cancellation,
    ) -> Result<PdfPage> {
        self.render_page(index, palette, Some(edge), cancel)
    }

    /// Return display pixels with the working memory reservation attached.
    pub fn display_frame(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        edge: u32,
        cancel: &Cancellation,
    ) -> Result<crate::media::RasterFrame> {
        self.display_frame_in_pool(index, palette, edge, cancel, pool())
    }

    fn display_frame_in_pool(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        edge: u32,
        cancel: &Cancellation,
        pool: &ResourcePool,
    ) -> Result<crate::media::RasterFrame> {
        let (pixels, memory) = self.rasterize(index, palette, Some(edge), cancel, pool)?;
        Ok(crate::media::RasterFrame::new(
            image::Frame::new(pixels),
            memory,
        ))
    }

    fn render_page(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        edge: Option<u32>,
        cancel: &Cancellation,
    ) -> Result<PdfPage> {
        let (pixels, _memory) = self.rasterize(index, palette, edge, cancel, pool())?;
        let mut png = Vec::new();
        PngEncoder::new(&mut png).write_image(
            &pixels,
            pixels.width(),
            pixels.height(),
            ExtendedColorType::Rgba8,
        )?;
        cancel.check()?;
        Ok(PdfPage {
            png,
            pages: self.page_count(),
        })
    }

    fn rasterize(
        &self,
        index: usize,
        palette: Option<ReaderPalette>,
        edge: Option<u32>,
        cancel: &Cancellation,
        pool: &ResourcePool,
    ) -> Result<(image::RgbaImage, WorkingReservation)> {
        let reading = ReadingColors {
            params: palette.map(|colors| ReadingParams {
                text: colors.ink,
                background: colors.paper,
                only_black: false,
                line_art: true,
            }),
        };
        cancel.check()?;
        let &(width, height) = self
            .sizes
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("PDF page is out of range"))?;
        ensure!(
            width.is_finite() && height.is_finite() && width > 0. && height > 0.,
            "Invalid PDF page dimensions"
        );
        let longest = width.max(height);
        let max_edge = crate::limits::IMAGE_PREVIEW_EDGE;
        let default_scale = (max_edge as f32 / longest).min(2.0);
        let scale = edge.map_or(default_scale, |edge| {
            effective_scale(width, height, edge.clamp(1, max_edge) as f32 / longest)
        });
        let fallback = scale.min(default_scale);
        let working_bytes = |scale| {
            u64::from(device_pixels(width, scale)) * u64::from(device_pixels(height, scale)) * 16
        };
        let required = working_bytes(scale);
        let minimum = working_bytes(fallback);
        let mut backend = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        cancel.check()?;
        let mut memory = pool.reserve_working(minimum, required)?;
        let scale = if memory.bytes() < required {
            memory.shrink_to(minimum);
            fallback
        } else {
            scale
        };
        let w = device_pixels(width, scale);
        let h = device_pixels(height, scale);
        let (width, height, data) = match &mut *backend {
            Backend::Buffered {
                renderer,
                bytes,
                colors,
                ..
            } => {
                if *colors != reading {
                    **renderer = PageRenderer::new(
                        bytes.clone(),
                        RenderConfig {
                            reading_colors: reading,
                            ..RenderConfig::default().on_screen()
                        },
                    );
                    *colors = reading;
                }
                let result = renderer.render(RenderRequest {
                    page: index,
                    scale,
                    ..Default::default()
                });
                cancel.check()?;
                if let Some(message) = result.error {
                    bail!("{message}");
                }
                (result.width, result.height, result.rgba.to_vec())
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
                        bg_color: reading.params.map_or(
                            hayro::vello_cpu::color::palette::css::WHITE,
                            |p| {
                                let [r, g, b] = p.background;
                                hayro::vello_cpu::color::AlphaColor::from_rgb8(r, g, b)
                            },
                        ),
                        color_replacement: reading.params.map(Into::into),
                        ..Default::default()
                    },
                );
                cancel.check()?;
                (
                    u32::from(pixmap.width()),
                    u32::from(pixmap.height()),
                    pixmap.data_as_u8_slice().to_vec(),
                )
            }
        };
        let pixels = image::RgbaImage::from_raw(width, height, data)
            .ok_or_else(|| anyhow::anyhow!("Invalid PDF raster dimensions"))?;
        Ok((pixels, memory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_frame_fits_without_png_decode_headroom() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/reader-contracts/reading-colors.pdf");
        let cancel = Cancellation::default();
        let doc = PdfDocument::open(&fixture, &cancel).unwrap();
        let budget = 20 * 1024 * 1024;
        let pool = ResourcePool::with_budgets(0, budget);
        let (frame, memory) = doc
            .display_frame_in_pool(0, None, 1024, &cancel, &pool)
            .unwrap()
            .into_bgra();
        let (width, height) = frame.buffer().dimensions();
        assert_eq!(width.max(height), 1024);
        assert_eq!(memory.bytes(), u64::from(width) * u64::from(height) * 4);
        assert!(pool.reserve_working(budget, budget).is_err());
        drop((frame, memory));
        assert!(pool.reserve_working(budget, budget).is_ok());
    }

    #[test]
    fn viewport_raster_uses_device_pixels_in_both_backends() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/reader-contracts/reading-colors.pdf");
        let cancel = Cancellation::default();
        let doc = PdfDocument::open(&fixture, &cancel).unwrap();
        let snapshot = Arc::new(MappedSnapshot::open(&fixture, &cancel).unwrap());
        let mapped = PdfDocument {
            sizes: doc.sizes.clone(),
            backend: Mutex::new(Backend::Mapped(Pdf::new(snapshot).unwrap())),
        };
        let (width, height) = doc.sizes[0];
        for edge in [512, 2304] {
            let scale = edge as f32 / width.max(height);
            let expected = (device_pixels(width, scale), device_pixels(height, scale));
            for backend in [&doc, &mapped] {
                let page = backend.render_at_edge(0, None, edge, &cancel).unwrap();
                let image = image::load_from_memory(&page.png).unwrap();
                assert_eq!((image.width(), image.height()), expected);
            }
        }
    }

    #[test]
    fn switching_reading_colors_repaints_the_same_page_and_restores_original() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/reader-contracts/reading-colors.pdf");
        let cancel = Cancellation::default();
        let doc = PdfDocument::open(&fixture, &cancel).unwrap();
        let before = doc.render(0, &cancel).unwrap();
        let palette = ReaderPalette {
            paper: [12, 18, 26],
            ink: [212, 218, 224],
            surface: [32; 3],
            accent: [120; 3],
        };
        let dark = doc.render_with_palette(0, Some(palette), &cancel).unwrap();
        let original = image::load_from_memory(&before.png).unwrap().to_rgb8();
        let rendered = image::load_from_memory(&dark.png).unwrap().to_rgb8();
        assert_eq!(original.dimensions(), rendered.dimensions());
        assert_eq!(original.get_pixel(0, 0).0, [255; 3]);
        assert_eq!(rendered.get_pixel(0, 0).0, palette.paper);
        for (x, y) in [(50, 170), (120, 170), (50, 215), (120, 215)] {
            assert_eq!(rendered.get_pixel(x, y), original.get_pixel(x, y));
        }
        assert!(rendered.pixels().any(|pixel| {
            pixel
                .0
                .iter()
                .zip(palette.ink)
                .all(|(a, b)| a.abs_diff(b) <= 2)
        }));
        assert_eq!(doc.render(0, &cancel).unwrap().png, before.png);

        // Exercise the bounded-memory route even on machines with enough room
        // to admit this tiny fixture to PDFCraft's buffered renderer.
        let snapshot = Arc::new(MappedSnapshot::open(&fixture, &cancel).unwrap());
        let mapped = PdfDocument {
            sizes: doc.sizes.clone(),
            backend: Mutex::new(Backend::Mapped(Pdf::new(snapshot).unwrap())),
        };
        let page = mapped
            .render_with_palette(0, Some(palette), &cancel)
            .unwrap();
        let from_disk = image::load_from_memory(&page.png).unwrap().to_rgb8();
        assert_eq!(from_disk.dimensions(), rendered.dimensions());
        assert_eq!(from_disk.get_pixel(0, 0), rendered.get_pixel(0, 0));
    }
}
