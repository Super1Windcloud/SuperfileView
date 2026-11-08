use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    component::{button::Button, h_flex, v_flex},
    *,
};
use std::{path::PathBuf, sync::Arc};
use superfile_services::{pdf::PdfDocument, work::Cancellation};

pub(super) struct PdfReader {
    path: PathBuf,
    document: Option<Arc<PdfDocument>>,
    image: Option<Entity<super::media::PreparedImage>>,
    task: Option<Task<()>>,
    page: usize,
    pages: usize,
    error: Option<String>,
}
impl PdfReader {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            path,
            document: None,
            image: None,
            task: None,
            page: 0,
            pages: 0,
            error: None,
        };
        view.load(0, cx);
        view
    }
    fn load(&mut self, page: usize, cx: &mut Context<Self>) {
        self.task = None;
        let path = self.path.clone();
        let document = self.document.clone();
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| {
                let document = match document {
                    Some(document) => document,
                    None => Arc::new(PdfDocument::open(&path, cancel)?),
                };
                let image = document.render(page, cancel)?;
                anyhow::Ok((document, image))
            },
        );
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok((document, image)) => {
                        if let Some(old) = view.image.take() {
                            old.update(cx, |image, cx| image.suspend(cx));
                        }
                        view.document = Some(document);
                        view.pages = image.pages;
                        view.page = page;
                        view.error = None;
                        view.image =
                            Some(cx.new(|cx| super::media::PreparedImage::new(image.png, cx)));
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
            if let Some(image) = &self.image {
                image.update(cx, |image, cx| image.suspend(cx));
            }
        } else if self.image.is_none() && self.task.is_none() && self.error.is_none() {
            self.load(self.page, cx);
        }
    }
}
impl Render for PdfReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use superfile_services::locale::tr;
        let mut body = v_flex().size_full().min_h_0();
        if let Some(image) = &self.image {
            let source = super::media::PreparedImage::source(image);
            let detail = source.clone();
            body = body.child(
                div()
                    .id("pdf-page")
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .on_click(move |event, window, cx| {
                        if event.click_count() == 2 {
                            super::image_detail::show(detail.clone(), window, cx);
                        }
                    })
                    .child(img(source).size_full().object_fit(ObjectFit::Contain)),
            );
        } else if self.task.is_some() {
            body = body.child(super::preview_loading(cx));
        }
        if let Some(error) = &self.error {
            body = body.child(div().p_4().child(error.clone())).child(
                Button::new("pdf-retry")
                    .label(tr("Retry"))
                    .on_click(cx.listener(|view, _, _, cx| view.load(view.page, cx))),
            );
        }
        let previous = self.page.checked_sub(1);
        let next = (self.page + 1 < self.pages).then_some(self.page + 1);
        let mut toolbar = h_flex().gap_3().p_2();
        if let Some(page) = previous {
            toolbar = toolbar.child(
                Button::new("pdf-previous")
                    .label(tr("Previous page"))
                    .on_click(cx.listener(move |view, _, _, cx| view.load(page, cx))),
            );
        }
        if self.pages > 0 {
            toolbar = toolbar.child(format!("{} / {}", self.page + 1, self.pages));
        }
        if let Some(page) = next {
            toolbar = toolbar.child(
                Button::new("pdf-next")
                    .label(tr("Next page"))
                    .on_click(cx.listener(move |view, _, _, cx| view.load(page, cx))),
            );
        }
        body.child(toolbar)
    }
}
