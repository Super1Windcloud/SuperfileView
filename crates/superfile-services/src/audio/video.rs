use anyhow::{Context, Result, ensure};
use deckcraft_media::{VideoFeed, probe_headers};
use std::{fs::File, io::Read, path::Path, sync::Arc, time::Duration};
use superfile_core::resources::WorkingReservation;

pub struct VideoFrame {
    pub frame: image::Frame,
    pub memory: WorkingReservation,
}
pub(super) struct Video {
    feed: VideoFeed,
    last: Option<f64>,
    pub duration: Duration,
    pub has_audio: bool,
    pub detail: String,
    _input: WorkingReservation,
    _decoder: WorkingReservation,
}
impl Video {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let input = crate::resources::pool().reserve_working(len, len)?;
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(usize::try_from(len)?)?;
        file.take(len).read_to_end(&mut buffer)?;
        ensure!(buffer.len() as u64 == len, "Video changed while opening");
        let bytes = Arc::new(buffer);
        let info = probe_headers(&bytes)?;
        let video = info.video.context("No video track")?;
        ensure!(video.decodable, "Unsupported video codec: {}", video.codec);
        // Feed buffers six decoded pictures ahead of the current and next frame.
        let needed = u64::from(video.width)
            .checked_mul(u64::from(video.height))
            .and_then(|pixels| pixels.checked_mul(4 * 12))
            .context("Invalid video dimensions")?;
        ensure!(needed > 0, "Missing video dimensions");
        let decoder = crate::resources::pool().reserve_working(needed, needed)?;
        let feed = VideoFeed::new(bytes)?;
        let detail = format!("{} · {} × {}", video.codec, video.width, video.height);
        Ok(Self {
            feed,
            last: None,
            has_audio: info.audio.is_some(),
            duration: Duration::from_millis(info.duration_ms),
            detail,
            _input: input,
            _decoder: decoder,
        })
    }
    pub fn seek(&mut self, duration: Duration) {
        self.feed.seek(duration.as_secs_f64());
        self.last = None;
    }
    pub fn ended(&self) -> bool {
        self.feed.ended()
    }
    pub fn frame(&mut self, time: Duration) -> Result<Option<VideoFrame>> {
        let Some(pic) = self.feed.frame_at(time.as_secs_f64()) else {
            ensure!(
                !self.feed.ended() || self.last.is_some(),
                "Cannot decode a picture from this video track"
            );
            return Ok(None);
        };
        if self.last == Some(pic.time) {
            return Ok(None);
        }
        self.last = Some(pic.time);
        let bytes = pic.rgba.len() as u64;
        let memory = crate::resources::pool().reserve_working(bytes, bytes)?;
        let mut bgra = pic.rgba.clone();
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let image = image::RgbaImage::from_raw(pic.width, pic.height, bgra)
            .context("Invalid video frame")?;
        Ok(Some(VideoFrame {
            frame: image::Frame::new(image),
            memory,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    // DeckCraft's CC0 moving RGB gradients (crates/media/tests/fixtures/make.sh).
    #[test]
    fn decodes_moving_video_and_seeks_back_to_the_first_picture() {
        let directory = super::super::tests::playback_samples();
        for (name, dimensions) in [
            ("gradient.mov", (64, 48)),
            ("gradient.mkv", (64, 64)),
            ("clip-h264.mp4", (64, 48)),
            ("clip-hevc.mp4", (64, 48)),
        ] {
            let file = directory.path().join(name);
            let mut video = Video::open(&file).unwrap();
            let picture = |video: &mut Video, time| {
                let started = Instant::now();
                loop {
                    if let Some(frame) = video.frame(time).unwrap() {
                        return frame.frame.into_buffer();
                    }
                    assert!(
                        started.elapsed() < Duration::from_secs(10),
                        "{name} frame timeout at {time:?}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            };
            let first = picture(&mut video, Duration::ZERO);
            assert_eq!(first.dimensions(), dimensions);
            let later = picture(&mut video, Duration::from_millis(400));
            assert_ne!(first, later);
            video.seek(Duration::ZERO);
            assert_eq!(first, picture(&mut video, Duration::ZERO));
        }
    }
}
