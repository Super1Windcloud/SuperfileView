use super::document::DocumentReader;
use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{component::button::Button, *};
use std::{path::PathBuf, sync::Arc};
use superfile_core::resources::TextSnapshot;

pub(super) struct OpenApiReader {
    // Release WebKit before its file-backed index and snapshot.
    view: Option<Entity<DocumentReader>>,
    index: Option<superfile_services::openapi::Index>,
    path: PathBuf,
    text: Option<Arc<TextSnapshot>>,
    task: Option<Task<()>>,
    error: Option<String>,
}
impl OpenApiReader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            text: None,
            task: None,
            view: None,
            index: None,
            error: None,
        }
    }
    pub fn input_snapshot(&self) -> Option<&Arc<TextSnapshot>> {
        self.text.as_ref()
    }
    pub fn load(&mut self, text: Option<Arc<TextSnapshot>>, cx: &mut Context<Self>) {
        let same = match (&self.text, &text) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new) || old.as_str() == new.as_str(),
            _ => false,
        };
        self.text = text;
        if same && self.index.is_some() {
            self.set_active(true, cx);
            return;
        }
        self.view = None;
        self.index = None;
        self.start(cx);
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.error = None;
        let path = self.path.clone();
        let text = self.text.clone();
        let (cancel, guard) = superfile_services::work::Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| {
                superfile_services::openapi::prepare(
                    &path,
                    text.as_ref().map(|s| s.as_str()),
                    cancel,
                )
            },
        );
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok(index) => {
                        view.view = Some(cx.new(|_| {
                            DocumentReader::table(view.path.clone(), index.database.clone())
                        }));
                        view.index = Some(index);
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if !active {
            self.task = None;
        }
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_active(active, cx));
        } else if active && self.task.is_none() && self.error.is_none() {
            if let Some(index) = &self.index {
                self.view = Some(
                    cx.new(|_| DocumentReader::table(self.path.clone(), index.database.clone())),
                );
                cx.notify();
            } else {
                self.start(cx);
            }
        }
    }
    pub fn find(&self, show: bool, cx: &App) -> bool {
        self.view.as_ref().is_some_and(|view| {
            let _ = show;
            view.read(cx).can_find()
        })
    }
}
impl Render for OpenApiReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = &self.view {
            return view.clone().into_any_element();
        }
        if let Some(error) = self.error.clone() {
            return div()
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(superfile_services::locale::tr(
                    "Cannot load API reference. Switch to Source to view or edit the document.",
                ))
                .child(div().text_sm().child(error))
                .child(
                    Button::new("retry-api-reference")
                        .label(superfile_services::locale::tr("Retry"))
                        .on_click(cx.listener(|view, _, _, cx| view.start(cx))),
                )
                .into_any_element();
        }
        super::preview_loading(cx)
    }
}
