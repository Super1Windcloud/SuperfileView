use gpui_kit::{
    component::{ActiveTheme, button::Button, h_flex, v_flex},
    *,
};
use std::path::PathBuf;
use superfile_services::{
    audio::{Command, Playback, Status},
    locale::tr,
};

pub(super) struct AudioReader {
    path: PathBuf,
    folder: bool,
    shuffle: bool,
    frame: Option<(
        std::sync::Arc<RenderImage>,
        superfile_core::resources::WorkingReservation,
    )>,
    commands: Option<std::sync::mpsc::Sender<Command>>,
    status: Status,
    listener: Option<Task<()>>,
}

impl AudioReader {
    pub fn new(
        path: PathBuf,
        folder: bool,
        shuffle: bool,
        autoplay: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let status = Status {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            ..Status::default()
        };
        let mut reader = Self {
            path,
            folder,
            shuffle,
            frame: None,
            commands: None,
            status,
            listener: None,
        };
        cx.on_release(|view, cx| view.clear_frame(cx)).detach();
        if autoplay {
            reader.start(cx);
        }
        reader
    }
    fn clear_frame(&mut self, cx: &mut App) {
        if let Some((image, _memory)) = self.frame.take() {
            cx.drop_image(image, None);
        }
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        match Playback::start(self.path.clone(), self.folder, self.shuffle, true) {
            Ok(worker) => {
                self.commands = Some(worker.commands);
                self.listener = Some(cx.spawn(async move |view, cx| {
                    while let Ok(update) = worker.updates.recv().await {
                        if view
                            .update(cx, |reader, cx| {
                                if reader.status.name != update.status.name
                                    || update.frame.is_some()
                                {
                                    reader.clear_frame(cx);
                                }
                                if let Some(frame) = update.frame {
                                    reader.frame = Some((
                                        std::sync::Arc::new(RenderImage::new(vec![frame.frame])),
                                        frame.memory,
                                    ));
                                }
                                reader.status = update.status;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = view.update(cx, |reader, cx| {
                        reader.commands = None;
                        cx.notify();
                    });
                }));
            }
            Err(error) => self.status.error = Some(error.to_string()),
        }
        cx.notify();
    }
    fn command(&self, cmd: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(cmd);
        }
    }
}
fn time(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Render for AudioReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut controls = h_flex().gap_2().items_center();
        if self.status.tracks > 1 {
            controls = controls.child(
                Button::new("previous-track")
                    .label(tr("Previous"))
                    .on_click(cx.listener(|this, _, _, _| this.command(Command::Previous))),
            );
        }
        controls = controls
            .child(
                Button::new("seek-back")
                    .label(tr("−10 s"))
                    .on_click(cx.listener(|this, _, _, _| this.command(Command::Seek(-10)))),
            )
            .child(
                Button::new("audio-toggle")
                    .label(if self.status.playing {
                        tr("Pause")
                    } else {
                        tr("Play")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.commands.is_some() {
                            this.command(Command::Toggle);
                        } else {
                            this.start(cx);
                        }
                    })),
            )
            .child(
                Button::new("seek-forward")
                    .label(tr("+10 s"))
                    .on_click(cx.listener(|this, _, _, _| this.command(Command::Seek(10)))),
            );
        if self.status.tracks > 1 {
            controls = controls.child(
                Button::new("next-track")
                    .label(tr("Next"))
                    .on_click(cx.listener(|this, _, _, _| this.command(Command::Next))),
            );
        }
        let pos = time(self.status.position);
        let clock = self.status.duration.map_or_else(
            || pos.clone(),
            |duration| format!("{pos} / {}", time(duration)),
        );
        let mut body = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(super::editor_paper(cx))
            .text_color(super::editor_ink(cx));
        if let Some((frame, _)) = &self.frame {
            body = body.child(
                div().flex_1().w_full().min_h_0().child(
                    img(frame.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                ),
            );
        }
        body = body
            .child(self.status.name.clone())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(clock),
            )
            .child(controls);
        if let Some(error) = &self.status.error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        body
    }
}

impl super::contract::ReaderView for super::surface::SurfaceEntity<AudioReader> {
    fn kind(&self) -> &'static str {
        "media"
    }
    fn render(&mut self, _: &mut App) -> AnyElement {
        self.0.clone().into_any_element()
    }
    fn set_active(&self, active: bool, cx: &mut App) {
        if !active {
            self.read(cx).command(Command::Pause);
        }
    }
    fn operations(&self) -> super::contract::ReaderOperations<'_> {
        super::contract::ReaderOperations {
            playback: Some(self),
            ..Default::default()
        }
    }
}
impl super::contract::PlaybackState for super::surface::SurfaceEntity<AudioReader> {
    fn playlist_shuffle(&self, cx: &App) -> Option<bool> {
        let view = self.read(cx);
        view.folder.then_some(view.shuffle)
    }
}
impl super::surface::ReaderSurface for super::surface::SurfaceEntity<AudioReader> {
    fn file_detail(&self, cx: &App) -> Option<String> {
        let status = &self.read(cx).status;
        if status.detail.is_empty() {
            status.duration.map(time)
        } else {
            Some(status.detail.clone())
        }
    }
}
