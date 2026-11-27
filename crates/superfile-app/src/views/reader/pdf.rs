use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    component::{button::Button, h_flex, scroll::Scrollbar, v_flex},
    prelude::FluentBuilder,
    *,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use superfile_core::reader_palette::ReaderPalette;
use superfile_services::{locale::tr, pdf::PdfDocument, work::Cancellation};

const PAGE_MARGIN_X: Pixels = px(16.);
const PAGE_MARGIN_Y: Pixels = px(8.);

enum Page {
    Loading { _task: Task<()> },
    Ready(Entity<super::media::RasterImage>),
    Failed(String),
}

pub(super) struct PdfReader {
    path: PathBuf,
    document: Option<Arc<PdfDocument>>,
    opening: Option<Task<()>>,
    error: Option<String>,
    list: ListState,
    pages: BTreeMap<usize, (u32, Page)>,
    viewport: Size<Pixels>,
    fit_page: bool,
    active: bool,
    follow_theme: bool,
    palette: Option<ReaderPalette>,
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
            viewport: size(px(0.), px(0.)),
            fit_page: false,
            active: true,
            follow_theme: true,
            palette: None,
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

    fn request(
        &mut self,
        index: usize,
        edge: u32,
        document: Arc<PdfDocument>,
        cx: &mut Context<Self>,
    ) {
        let palette = self.palette;
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Viewport,
            cancel,
            move |cancel| document.display_frame(index, palette, edge, cancel),
        );
        let task = cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                if palette != view.palette
                    || !matches!(view.pages.get(&index), Some((target, Page::Loading { .. })) if *target == edge)
                {
                    return;
                }
                let page = match result {
                    Ok(image) => {
                        Page::Ready(cx.new(|cx| super::media::RasterImage::new(image, cx)))
                    }
                    Err(error) => Page::Failed(format!("{error:#}")),
                };
                view.pages.insert(index, (edge, page));
                cx.notify();
            });
        });
        self.pages
            .insert(index, (edge, Page::Loading { _task: task }));
    }

    fn row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = self.document.clone() else {
            return div().into_any_element();
        };
        let ratio = doc.page_ratio(index).unwrap_or(0.75);
        let available_width = (self.viewport.width - PAGE_MARGIN_X * 2.).max(px(1.));
        let available_height = (self.viewport.height - PAGE_MARGIN_Y * 2.).max(px(1.));
        let width = if self.fit_page {
            available_width.min(available_height * ratio)
        } else {
            available_width
        };
        let height = width / ratio;
        let device_edge = f32::from(width.max(height)) * window.scale_factor();
        // Round up by a small step so adjacent sizes share a raster during resize.
        let edge = ((device_edge / 128.).ceil() * 128.) as u32;
        let edge = edge.clamp(1, superfile_services::limits::IMAGE_PREVIEW_EDGE);
        if self.active
            && self
                .pages
                .get(&index)
                .is_none_or(|(target, _)| *target != edge)
        {
            self.request(index, edge, doc, cx);
        }
        let mut paper = div()
            .id(("pdf-page", index))
            .relative()
            .w(width)
            .h(height)
            .flex_none();
        match self.pages.get(&index).map(|(_, page)| page) {
            Some(Page::Ready(image)) => {
                let source = super::media::RasterImage::source(image);
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
        h_flex()
            .w_full()
            .py(PAGE_MARGIN_Y)
            .justify_center()
            .when(self.fit_page, |row| row.min_h(self.viewport.height))
            .child(paper)
            .into_any_element()
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
        let editor = super::reader_palette(cx);
        let palette = self.follow_theme.then_some(editor);
        if self.palette != palette {
            self.pages.clear();
            self.palette = palette;
        }
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
            let mut toolbar = h_flex().flex_wrap().gap_3().p_2();
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
            toolbar = toolbar.child(
                Button::new("pdf-fit")
                    .label(if self.fit_page {
                        tr("Fit width")
                    } else {
                        tr("Fit page")
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        let current = view.current();
                        view.fit_page = !view.fit_page;
                        view.list.remeasure();
                        view.jump(current, cx);
                    })),
            );
            toolbar = toolbar.child(
                Button::new("pdf-reading-colors")
                    .label(if self.follow_theme {
                        tr("Show original colors")
                    } else {
                        tr("Use editor colors")
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.follow_theme = !view.follow_theme;
                        cx.notify();
                    })),
            );
            let reader = cx.entity().downgrade();
            body = body
                .child(
                    div()
                        .relative()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        // Capture the reader pane after layout, before the list measures its pages.
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    let _ = reader.update(cx, |view, _| {
                                        if view.viewport != bounds.size {
                                            if view.viewport.width != bounds.size.width
                                                || view.fit_page
                                            {
                                                view.list.remeasure();
                                            }
                                            view.viewport = bounds.size;
                                        }
                                    });
                                },
                                |_, (), _, _| {},
                            )
                            .absolute()
                            .inset_0()
                            .size_full(),
                        )
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
        cx.update_window(handle, |_, window, cx| {
            use gpui_kit::component::{Theme, ThemeMode};
            assert!(matches!(
                reader.read(cx).pages.get(&0),
                Some((_, Page::Ready(_)))
            ));
            Theme::change(ThemeMode::Dark, None, cx);
            window.render_frame(cx);
            let offset = reader.read(cx).list.logical_scroll_top();
            assert!(reader.read(cx).palette.is_some());
            window.click("pdf-reading-colors", cx);
            window.render_frame(cx);
            assert!(reader.read(cx).palette.is_none());

            window.click("pdf-reading-colors", cx);
            window.render_frame(cx);
            assert!(reader.read(cx).palette.is_some());
            let after = reader.read(cx).list.logical_scroll_top();
            assert_eq!(offset.item_ix, after.item_ix);
            assert_eq!(offset.offset_in_item, after.offset_in_item);
            Theme::change(ThemeMode::Light, None, cx);
            Theme::update(cx, |theme| {
                std::sync::Arc::make_mut(&mut theme.highlight_theme)
                    .style
                    .editor_background = Some(gpui_kit::rgb(0xfdf6e3).into());
                std::sync::Arc::make_mut(&mut theme.highlight_theme)
                    .style
                    .editor_foreground = Some(gpui_kit::rgb(0x586e75).into());
            });
            window.render_frame(cx);
            let themed = reader.read(cx).palette.unwrap();
            assert_eq!(themed.paper, [0xfd, 0xf6, 0xe3]);
            assert_eq!(themed.ink, [0x58, 0x6e, 0x75]);
            window.click("pdf-reading-colors", cx);
            window.render_frame(cx);
            assert!(reader.read(cx).palette.is_none());

            let width_edge = reader.read(cx).pages.get(&0).unwrap().0;
            window.click("pdf-fit", cx);
            window.render_frame(cx);
            let view = reader.read(cx);
            assert_eq!(view.current(), 0);
            assert!(view.fit_page);
            assert!(view.pages.get(&0).unwrap().0 < width_edge);
            for page in 1..view.page_count() {
                window.click("pdf-next", cx);
                window.render_frame(cx);
                let view = reader.read(cx);
                let viewport = view.list.viewport_bounds();
                let row = view.list.bounds_for_item(page).unwrap();
                assert_eq!(view.current(), page);
                assert!(row.top() >= viewport.top());
                assert!(row.bottom() <= viewport.bottom());
            }
        })
        .unwrap();
    }
}
