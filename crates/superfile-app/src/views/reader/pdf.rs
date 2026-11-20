use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    component::{button::Button, h_flex, scroll::Scrollbar, v_flex},
    *,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use superfile_services::{locale::tr, pdf::PdfDocument, work::Cancellation};

enum Page {
    Loading { _task: Task<()> },
    Ready(Entity<super::media::PreparedImage>),
    Failed(String),
}

pub(super) struct PdfReader {
    path: PathBuf,
    document: Option<Arc<PdfDocument>>,
    opening: Option<Task<()>>,
    error: Option<String>,
    list: ListState,
    pages: BTreeMap<usize, Page>,
    active: bool,
}

impl PdfReader {
    pub(super) fn page_count(&self) -> usize {
        self.document.as_ref().map_or(0, |doc| doc.page_count())
    }

    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(64.));
        let mut view = Self {
            path,
            document: None,
            opening: None,
            error: None,
            list,
            pages: BTreeMap::new(),
            active: true,
        };
        view.open(cx);
        view
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let path = self.path.clone();
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| PdfDocument::open(&path, cancel),
        );
        self.opening = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.opening = None;
                match result {
                    Ok(document) => {
                        view.list.reset(document.page_count());
                        view.document = Some(Arc::new(document));
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn request(&mut self, index: usize, document: Arc<PdfDocument>, cx: &mut Context<Self>) {
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Viewport,
            cancel,
            move |cancel| document.render(index, cancel),
        );
        let task = cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                let page = match result {
                    Ok(image) => {
                        Page::Ready(cx.new(|cx| super::media::PreparedImage::new(image.png, cx)))
                    }
                    Err(error) => Page::Failed(format!("{error:#}")),
                };
                view.pages.insert(index, page);
                cx.notify();
            });
        });
        self.pages.insert(index, Page::Loading { _task: task });
    }

    fn row(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = self.document.clone() else {
            return div().into_any_element();
        };
        let ratio = doc.page_ratio(index).unwrap_or(0.75);
        if self.active && !self.pages.contains_key(&index) {
            self.request(index, doc, cx);
        }
        let mut paper = div()
            .id(("pdf-page", index))
            .relative()
            .w_full()
            .aspect_ratio(ratio);
        match self.pages.get(&index) {
            Some(Page::Ready(image)) => {
                let source = super::media::PreparedImage::source(image);
                let detail = source.clone();
                paper = paper
                    .on_click(move |event, window, cx| {
                        if event.click_count() == 2 {
                            super::image_detail::show(detail.clone(), window, cx);
                        }
                    })
                    .child(
                        img(source)
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    );
            }
            Some(Page::Failed(error)) => {
                paper = paper.child(
                    v_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(error.clone())
                        .child(
                            Button::new(("pdf-retry", index))
                                .label(tr("Retry"))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.pages.remove(&index);
                                    cx.notify();
                                })),
                        ),
                );
            }
            _ => paper = paper.child(div().absolute().inset_0().child(super::preview_loading(cx))),
        }
        div().w_full().px_4().py_2().child(paper).into_any_element()
    }

    fn current(&self) -> usize {
        self.list
            .logical_scroll_top()
            .item_ix
            .min(self.page_count().saturating_sub(1))
    }

    fn jump(&mut self, index: usize, cx: &mut Context<Self>) {
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        self.pages.retain(|page, _| page.abs_diff(index) <= 1);
        cx.notify();
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.active = active;
        if !active {
            self.opening = None;
            self.pages.clear();
        } else if self.document.is_none() && self.opening.is_none() && self.error.is_none() {
            self.open(cx);
        }
        cx.notify();
    }
}

impl Render for PdfReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex().size_full().min_h_0();
        let count = self.page_count();
        if count > 0 {
            let current = self.current();
            let scroll = &self.list;
            let bottom = scroll.viewport_bounds().bottom() + px(64.);
            self.pages.retain(|&index, _| {
                index.abs_diff(current) <= 1
                    || (index > current
                        && scroll
                            .bounds_for_item(index - 1)
                            .is_some_and(|bounds| bounds.top() <= bottom))
            });
            let mut toolbar = h_flex().gap_3().p_2();
            if current > 0 {
                toolbar = toolbar.child(
                    Button::new("pdf-previous")
                        .label(tr("Previous page"))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.jump(view.current().saturating_sub(1), cx)
                        })),
                );
            }
            toolbar = toolbar.child(format!("{} / {}", current + 1, count));
            if current + 1 < count {
                toolbar = toolbar.child(Button::new("pdf-next").label(tr("Next page")).on_click(
                    cx.listener(|view, _, _, cx| {
                        view.jump((view.current() + 1).min(view.page_count() - 1), cx)
                    }),
                ));
            }
            body = body
                .child(
                    div()
                        .relative()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .child(list(self.list.clone(), cx.processor(Self::row)).size_full())
                        .child(Scrollbar::vertical(&self.list)),
                )
                .child(toolbar.flex_shrink_0());
        } else if let Some(error) = &self.error {
            body = body.child(div().p_4().child(error.clone())).child(
                Button::new("pdf-open-retry")
                    .label(tr("Retry"))
                    .on_click(cx.listener(|view, _, _, cx| view.open(cx))),
            );
        } else {
            body = body.child(super::preview_loading(cx));
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{Page, PdfReader};
    use gpui_kit::{
        App, AppContext, Bounds, PlatformInput, ScrollDelta, ScrollWheelEvent, TestAppContext,
        TouchPhase, Window, WindowBounds, WindowOptions, point, px, size, test::TestWindowExt,
    };
    use std::path::PathBuf;

    #[gpui_kit::test]
    fn wheel_scrolls_between_different_page_sizes_and_back(cx: &mut TestAppContext) {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/continuous-pages.pdf");
        cx.update(gpui_kit::init);
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(480.), px(360.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|cx| PdfReader::new(path, cx)),
            )
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = reader.read(cx);
            assert_eq!(view.page_count(), 4);
            let document = view.document.as_ref().unwrap();
            assert_eq!(document.page_ratio(2), Some(1.5));
            let first = view.list.bounds_for_item(0).unwrap();
            let viewport = view.list.viewport_bounds();
            assert!(first.size.height > viewport.size.height);
            let pos = viewport.center();
            let scroll = |dy, window: &mut Window, cx: &mut App| {
                window.dispatch_event(
                    PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position: pos,
                        delta: ScrollDelta::Pixels(point(px(0.), dy)),
                        touch_phase: TouchPhase::Moved,
                        ..Default::default()
                    }),
                    cx,
                );
                window.render_frame(cx);
            };
            // A wheel gesture produces a sequence of pixel deltas; newly visible
            // rows are measured between frames as the first sheet approaches its end.
            for _ in 0..(f32::from(first.size.height) / 40.) as usize - 1 {
                scroll(px(-40.), window, cx);
            }
            let top = reader.read(cx).list.logical_scroll_top();
            assert_eq!(top.item_ix, 0);
            assert!(top.offset_in_item > px(0.));
            let next = reader.read(cx).list.bounds_for_item(1).unwrap();
            assert!(
                next.top() > viewport.top() && next.top() < viewport.bottom(),
                "expected two pages in {viewport:?}, second page is at {next:?}, scroll is {top:?}"
            );
            scroll(px(-130.), window, cx);
            assert_eq!(reader.read(cx).current(), 1);
            scroll(px(150.), window, cx);
            assert_eq!(reader.read(cx).current(), 0);
            window.click("pdf-next", cx);
            assert_eq!(reader.read(cx).current(), 1);
            window.click("pdf-previous", cx);
            assert_eq!(reader.read(cx).current(), 0);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(matches!(
                reader.read(cx).pages.get(&0),
                Some(Page::Ready(_))
            ))
        });
    }
}
