use gpui_kit::{
    component::{ActiveTheme, Disableable, button::Button, h_flex, v_flex},
    *,
};
use std::path::PathBuf;
use superfile_services::{instance::Instance, locale::tr};

pub(super) fn run(instance: Instance, urls: Vec<String>, error: String) -> anyhow::Result<()> {
    let executable = std::env::current_exe()?;
    let directory = superfile_services::platform::data_directory()?;
    superfile_services::locale::set("system");
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        #[cfg(target_os = "macos")]
        super::icon::set();
        gpui_kit::component::set_locale(superfile_services::locale::code());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(620.), px(300.)),
                cx,
            ))),
            ..Default::default()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("Superfile");
            cx.new(|_| Recovery {
                _instance: instance,
                urls,
                directory,
                executable,
                error,
                check: None,
            })
        }) {
            log::error!("Cannot open application data recovery window: {error:#}");
            cx.quit();
        } else {
            cx.activate(true);
        }
    });
    Ok(())
}

struct Recovery {
    _instance: Instance,
    urls: Vec<String>,
    directory: PathBuf,
    executable: PathBuf,
    error: String,
    check: Option<Task<()>>,
}

impl Recovery {
    fn retry(&mut self, cx: &mut Context<Self>) {
        if self.check.is_some() {
            return;
        }
        let work = cx
            .background_executor()
            .spawn(async { superfile_services::storage::initialize() });
        self.check = Some(cx.spawn(async move |view, cx| {
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.check.take();
                let result = result.and_then(|()| {
                    superfile_services::cli::spawn_detached(&view.executable, &view.urls)
                });
                match result {
                    Ok(()) => cx.quit(),
                    Err(error) => {
                        view.error = format!("{error:#}");
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
}

impl Render for Recovery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let directory = self.directory.clone();
        let heading = div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .child(tr("Superfile could not open its application data"));
        let details = div()
            .id("startup-recovery-details")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(self.error.clone());
        let buttons = h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("startup-data-folder")
                    .label(tr("Open Folder"))
                    .on_click(move |_, _, cx| cx.reveal_path(&directory)),
            )
            .child(
                Button::new("startup-retry")
                    .label(tr("Retry"))
                    .disabled(self.check.is_some())
                    .on_click(cx.listener(|view, _, _, cx| view.retry(cx))),
            );
        v_flex()
            .id("startup-recovery")
            .size_full()
            .gap_3()
            .p_5()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(heading)
            .child(tr("The existing database has been kept. Open its folder to back up or restore it, then retry."))
            .child(details)
            .child(buttons)
    }
}
