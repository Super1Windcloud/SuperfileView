//! Keep stream initialization, decoder I/O, and seeks off the UI thread.
mod video;
use anyhow::{Context, Result, ensure};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};
pub use video::VideoFrame;

pub fn supports(path: &Path) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "aac" | "aif" | "aiff" | "caf" | "wav" | "m4a"
            )
        })
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub name: String,
    pub position: u64,
    pub duration: Option<u64>,
    pub playing: bool,
    pub tracks: usize,
    pub detail: String,
    pub error: Option<String>,
}

pub enum Command {
    Toggle,
    Pause,
    Next,
    Previous,
    Seek(i64),
}

pub struct Update {
    pub status: Status,
    pub frame: Option<VideoFrame>,
}

pub struct Playback {
    pub commands: Sender<Command>,
    pub updates: async_channel::Receiver<Update>,
}

impl Playback {
    pub fn start(path: PathBuf, folder: bool, shuffle: bool, autoplay: bool) -> Result<Self> {
        let (commands, input) = mpsc::channel();
        let (output, updates) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || {
                if let Err(error) = run(&path, folder, shuffle, autoplay, &input, &output) {
                    let _ = output.force_send(Update {
                        status: Status {
                            error: Some(format!("{error:#}")),
                            ..Status::default()
                        },
                        frame: None,
                    });
                }
            })?;
        Ok(Self { commands, updates })
    }
}

fn playlist(path: &Path, folder: bool, shuffle: bool) -> Result<Vec<PathBuf>> {
    if !folder {
        return Ok(vec![path.to_owned()]);
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if crate::preview::is_media(&entry.path()) && entry.file_type()?.is_file() {
            paths.push(entry.path());
        }
    }
    ensure!(
        !paths.is_empty(),
        "No supported audio tracks in this folder"
    );
    paths.sort();
    if shuffle {
        fastrand::shuffle(&mut paths);
    }
    Ok(paths)
}

struct Sound {
    player: Player,
    _device: MixerDeviceSink,
    duration: Option<Duration>,
}
impl Sound {
    fn open(path: &Path) -> Result<Self> {
        let source =
            Decoder::try_from(File::open(path)?).context("Cannot decode this audio track")?;
        let duration = source.total_duration();
        let mut device = DeviceSinkBuilder::open_default_sink()
            .context("Cannot open the audio output device")?;
        device.log_on_drop(false);
        let player = Player::connect_new(device.mixer());
        player.append(source);
        Ok(Self {
            player,
            _device: device,
            duration,
        })
    }
}

struct Track {
    sound: Option<Sound>,
    video: Option<video::Video>,
    duration: Option<Duration>,
    offset: Duration,
    running: Option<Instant>,
    warning: Option<String>,
}
impl Track {
    fn open(path: &Path) -> Result<Self> {
        let video = if supports(path) {
            None
        } else {
            Some(video::Video::open(path)?)
        };
        let mut warning = None;
        let sound = if video.as_ref().is_none_or(|v| v.has_audio) {
            match Sound::open(path) {
                Ok(sound) => Some(sound),
                Err(error) if video.is_some() => {
                    warning = Some(format!("{error:#}"));
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let duration = video
            .as_ref()
            .map(|v| v.duration)
            .filter(|d| !d.is_zero())
            .or_else(|| sound.as_ref().and_then(|s| s.duration));
        Ok(Self {
            sound,
            video,
            duration,
            offset: Duration::ZERO,
            running: Some(Instant::now()),
            warning,
        })
    }
    fn position(&self) -> Duration {
        if let Some(sound) = &self.sound
            && !sound.player.empty()
        {
            return sound.player.get_pos();
        }
        self.offset + self.running.map_or(Duration::ZERO, |t| t.elapsed())
    }
    fn pause(&mut self) {
        self.offset = self.position();
        self.running = None;
        if let Some(sound) = &self.sound {
            sound.player.pause();
        }
    }
    fn resume(&mut self) {
        self.running = Some(Instant::now());
        if let Some(sound) = &self.sound {
            sound.player.play();
        }
    }
    fn seek(&mut self, delta: i64) -> Result<()> {
        let next = Duration::from_secs(self.position().as_secs().saturating_add_signed(delta))
            .min(self.duration.unwrap_or(Duration::MAX));
        if let Some(sound) = &self.sound {
            sound.player.try_seek(next)?;
        }
        if let Some(video) = &mut self.video {
            video.seek(next);
        }
        self.offset = next;
        self.running = self.running.map(|_| Instant::now());
        Ok(())
    }
    fn finished(&self) -> bool {
        self.sound.as_ref().is_none_or(|s| s.player.empty())
            && self
                .video
                .as_ref()
                .is_none_or(|v| v.ended() && self.duration.is_none_or(|d| self.position() >= d))
    }
}
fn run(
    path: &Path,
    folder: bool,
    shuffle: bool,
    autoplay: bool,
    commands: &Receiver<Command>,
    updates: &async_channel::Sender<Update>,
) -> Result<()> {
    let paths = playlist(path, folder, shuffle)?;
    let mut index = 0;
    let mut track: Option<Track> = None;
    let mut open = autoplay;
    let mut error = None;
    let mut last = None;
    loop {
        if open {
            track = None;
            match Track::open(&paths[index]) {
                Ok(next) => {
                    error = next.warning.clone();
                    track = Some(next);
                }
                Err(e) => error = Some(format!("{e:#}")),
            }
            open = false;
        }
        let mut frame = None;
        if let Some(t) = &mut track {
            let pos = t.position();
            if let Some(video) = &mut t.video {
                frame = video.frame(pos)?;
            }
        }
        let finished = track.as_ref().is_some_and(Track::finished);
        if finished && index + 1 < paths.len() {
            index += 1;
            open = true;
            continue;
        }
        let status = Status {
            tracks: paths.len(),
            name: paths[index]
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            playing: track
                .as_ref()
                .is_some_and(|t| !finished && t.running.is_some()),
            position: track.as_ref().map_or(0, |t| {
                if finished {
                    t.duration.unwrap_or_default().as_secs()
                } else {
                    t.position().as_secs()
                }
            }),
            duration: track.as_ref().and_then(|t| t.duration).map(|v| v.as_secs()),
            detail: track
                .as_ref()
                .and_then(|t| t.video.as_ref())
                .map(|v| v.detail.clone())
                .unwrap_or_default(),
            error: error.clone(),
        };
        if frame.is_some() || last.as_ref() != Some(&status) {
            if updates
                .force_send(Update {
                    status: status.clone(),
                    frame,
                })
                .is_err()
            {
                break;
            }
            last = Some(status);
        }
        let wait = if track
            .as_ref()
            .is_some_and(|t| t.video.is_some() && !finished && t.running.is_some())
        {
            15
        } else {
            200
        };
        let command = match commands.recv_timeout(Duration::from_millis(wait)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
        };
        match command {
            Command::Toggle => match &mut track {
                Some(t) if !finished && t.running.is_none() => t.resume(),
                Some(t) if !finished => t.pause(),
                _ => open = true,
            },
            Command::Pause => {
                if let Some(t) = &mut track {
                    t.pause();
                }
            }
            Command::Next | Command::Previous => {
                index = if matches!(command, Command::Next) {
                    (index + 1) % paths.len()
                } else {
                    (index + paths.len() - 1) % paths.len()
                };
                open = true;
            }
            Command::Seek(delta) => {
                if let Some(t) = &mut track
                    && let Err(e) = t.seek(delta)
                {
                    error = Some(e.to_string());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn playback_samples() -> tempfile::TempDir {
        // The compressed tones and gradients are the DeckCraft media suite's CC0 fixtures.
        let files = include_bytes!("../../../assets/reader-contracts/playback.zip");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&files[..])).unwrap();
        let dir = tempfile::tempdir().unwrap();
        zip.extract(dir.path()).unwrap();
        for name in ["gradient.mkv", "gradient.mov"] {
            let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/reader-contracts")
                .join(name);
            std::fs::copy(sample, dir.path().join(name)).unwrap();
        }
        dir
    }
    #[test]
    #[ignore = "requires a configured desktop audio output"]
    fn native_device_plays_silent_wav_to_completion() {
        let dir = playback_samples();
        let path = dir.path().join("tone.wav");
        let mut pcm = std::fs::read(&path).unwrap();
        let data = pcm.windows(4).position(|w| w == b"data").unwrap() + 8;
        pcm[data..].fill(0);
        std::fs::write(&path, pcm).unwrap();
        let sound = Sound::open(&path).unwrap();
        assert!(sound.duration.is_some());
        let start = Instant::now();
        while !sound.player.empty() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "audio device stopped consuming samples"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(start.elapsed() >= Duration::from_millis(5));
    }
    #[test]
    fn decodes_audio_catalogue_to_pcm_without_a_device() {
        let dir = playback_samples();
        for name in [
            "tone.aac",
            "tone.aif",
            "tone.aiff",
            "tone.wav",
            "tone.caf",
            "tone.mp3",
            "tone.m4a",
            "tone-alac.m4a",
            "tone.flac",
        ] {
            let path = dir.path().join(name);
            assert!(supports(&path));
            let decoder = Decoder::try_from(File::open(path).unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(decoder.take(4096).any(|s| s.abs() > 0.005), "{name}");
        }
        std::fs::write(dir.path().join("ignore.md"), "text").unwrap();
        let sorted = playlist(dir.path(), true, false).unwrap();
        assert_eq!(sorted.len(), 13);
        assert!(sorted.windows(2).all(|w| w[0] < w[1]));
        let mut shuffled = playlist(dir.path(), true, true).unwrap();
        shuffled.sort();
        assert_eq!(sorted, shuffled);
    }
}
