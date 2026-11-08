//! OpenSSH authentication uses a small GPUI window on every platform.
use gpui_kit::{
    component::{
        TitleBar,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    *,
};
use std::{cell::RefCell, rc::Rc};

struct Prompt {
    question: String,
    confirmation: bool,
    input: Entity<InputState>,
    answer: Rc<RefCell<Option<String>>>,
    _submit: Subscription,
}
impl Prompt {
    fn submit(&mut self, cx: &mut Context<Self>) {
        *self.answer.borrow_mut() = Some(if self.confirmation {
            "yes".into()
        } else {
            self.input.read(cx).value().to_string()
        });
        cx.quit();
    }
}
impl Render for Prompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use superfile_services::locale::tr;
        v_flex()
            .size_full()
            .child(TitleBar::new().child("Superfile · SSH"))
            .child(
                v_flex()
                    .flex_1()
                    .p_4()
                    .gap_4()
                    .child(
                        div()
                            .id("ssh-question")
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .whitespace_normal()
                            .child(self.question.clone()),
                    )
                    .children((!self.confirmation).then(|| Input::new(&self.input)))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .label(tr("Cancel"))
                                    .on_click(|_, _, cx| cx.quit()),
                            )
                            .child(
                                Button::new("confirm")
                                    .primary()
                                    .label(tr("Connect"))
                                    .on_click(cx.listener(|view, _, _, cx| view.submit(cx))),
                            ),
                    ),
            )
    }
}
pub(super) fn run() -> anyhow::Result<()> {
    superfile_services::locale::initialize();
    let question = std::env::args().nth(1).unwrap_or_default();
    let confirmation = std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm")
        || question.contains("yes/no");
    let answer = Rc::new(RefCell::new(None));
    let selected = answer.clone();
    gpui_kit::application()
        .with_assets(super::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_quit_mode(QuitMode::LastWindowClosed);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(480.), px(320.)),
                    cx,
                ))),
                ..TitleBar::window_options()
            };
            if let Err(error) = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| {
                    let input = cx.new(|cx| InputState::new(window, cx).masked(!confirmation));
                    let submit =
                        cx.subscribe_in(&input, window, |view: &mut Prompt, _, event, _, cx| {
                            if matches!(event, InputEvent::PressEnter { .. }) {
                                view.submit(cx);
                            }
                        });
                    input.update(cx, |input, cx| input.focus(window, cx));
                    Prompt {
                        question,
                        confirmation,
                        input,
                        answer: selected,
                        _submit: submit,
                    }
                })
            }) {
                eprintln!("Cannot open SSH authentication: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
    let selected = answer
        .borrow_mut()
        .take()
        .ok_or_else(|| anyhow::anyhow!("SSH authentication cancelled"))?;
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    writeln!(output, "{selected}")?;
    output.flush()?;
    Ok(())
}
