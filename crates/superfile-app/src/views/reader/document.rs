//! Documents rendered with the same GPUI components on every desktop platform.
mod word;

use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    base::{TextView, TextViewState, TextViewStyle},
    component::{ActiveTheme, Disableable, button::Button, h_flex, v_flex},
    *,
};
use std::path::PathBuf;
use superfile_services::{
    rich_documents::{self, LayoutFallback, PreparedDocument},
    work::Cancellation,
};

pub(super) struct DocumentReader {
    path: PathBuf,
    folder: bool,
    shuffle: bool,
    task: Option<Task<()>>,
    prepared: Option<PreparedDocument>,
    state: Option<Entity<TextViewState>>,
    word: Option<word::WordView>,
    table: Option<Entity<super::sqlite::SqliteReader>>,
    pdf: Option<Entity<super::pdf::PdfReader>>,
    page_file: Option<rich_documents::DocumentPages>,
    reading_text: bool,
    layout_fallback: Option<LayoutFallback>,
    layout_retry: Option<Task<()>>,
    database: Option<PathBuf>,
    find: Option<Entity<super::find::FindBar>>,
    error: Option<String>,
    history: Vec<u64>,
    html_open: Option<super::OpenLink>,
    _subscription: Option<Subscription>,
}
impl DocumentReader {
    fn empty(path: PathBuf) -> Self {
        Self {
            path,
            folder: false,
            shuffle: false,
            task: None,
            prepared: None,
            state: None,
            word: None,
            table: None,
            pdf: None,
            page_file: None,
            reading_text: false,
            layout_fallback: None,
            layout_retry: None,
            database: None,
            find: None,
            error: None,
            history: Vec::new(),
            html_open: None,
            _subscription: None,
        }
    }
    pub fn html(
        path: PathBuf,
        text: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut reader = Self::empty(path);
        if let Some(text) = text {
            reader.set_text(&text, true, cx);
        } else {
            reader.open(cx);
        }
        reader
    }
    pub fn table(path: PathBuf, database: PathBuf) -> Self {
        let mut reader = Self::empty(path);
        reader.database = Some(database);
        reader
    }
    pub fn new(path: PathBuf, folder: bool, shuffle: bool, cx: &mut Context<Self>) -> Self {
        let mut reader = Self::empty(path);
        if reader
            .path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            reader.pdf = Some(cx.new(|cx| super::pdf::PdfReader::new(reader.path.clone(), cx)));
        }
        reader.folder = folder;
        reader.shuffle = shuffle;
        reader.open(cx);
        reader
    }
    pub fn new_paused(path: PathBuf, folder: bool, shuffle: bool, cx: &mut Context<Self>) -> Self {
        Self::new(path, folder, shuffle, cx)
    }
    fn set_text(&mut self, text: &str, html: bool, cx: &mut Context<Self>) {
        let (marked, _) = superfile_services::markup_highlights::preview(text, html);
        let text = marked.as_ref();
        if let Some(state) = &self.state {
            state.update(cx, |state, cx| state.set_text(text, cx));
        } else {
            let state = cx.new(|cx| {
                if html {
                    TextViewState::html(text, cx)
                } else {
                    TextViewState::markdown(text, cx)
                }
            });
            self._subscription = Some(cx.observe(&state, |_, _, cx| cx.notify()));
            self.state = Some(state);
        }
    }
    pub fn reload_html_file(&mut self, cx: &mut Context<Self>) {
        self.prepared = None;
        self.state = None;
        self.open(cx);
    }
    pub fn set_html_open(&mut self, open: super::OpenLink) {
        self.html_open = Some(open);
    }
    pub fn reveal_html_anchor(&mut self, _anchor: &str) {}
    pub fn playlist_shuffle(&self) -> Option<bool> {
        self.folder.then_some(self.shuffle)
    }
    pub fn can_find(&self) -> bool {
        self.word.is_some() || (self.state.is_some() && (self.pdf.is_none() || self.reading_text))
    }
    pub(super) fn can_move_to_window(&self, _cx: &App) -> bool {
        self.task.is_none() && self.layout_retry.is_none()
    }
    pub(super) fn hide(&mut self) {
        self.task = None;
        self.layout_retry = None;
    }
    pub(super) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if let Some(pdf) = &self.pdf
            && !self.reading_text
        {
            pdf.update(cx, |pdf, cx| pdf.set_active(active, cx));
            return;
        }
        if active {
            self.open(cx);
        } else {
            self.hide();
        }
    }
    pub(super) fn open(&mut self, cx: &mut Context<Self>) {
        if self.pdf.is_some() && !self.reading_text {
            return;
        }
        if self.task.is_some()
            || self.prepared.is_some()
            || self.state.is_some()
            || self.word.is_some()
            || self.database.is_some()
        {
            return;
        }
        self.error = None;
        let path = self.path.clone();
        let reading_text = self.reading_text;
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| {
                if reading_text {
                    rich_documents::read_document_text(&path, cancel)
                } else {
                    rich_documents::prepare(&path, cancel)
                }
            },
        );
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok(PreparedDocument::Word(document)) => view.show_word(document, cx),
                    Ok(PreparedDocument::Pages(path)) => {
                        view.show_layout(path, cx);
                    }
                    Ok(PreparedDocument::FallbackText { page, reason }) => {
                        view.layout_fallback = Some(reason);
                        view.set_text(&page.text, false, cx);
                        view.prepared = Some(PreparedDocument::Text { page, html: false });
                    }
                    Ok(PreparedDocument::Text { page, html }) => {
                        view.set_text(&page.text, html, cx);
                        view.prepared = Some(PreparedDocument::Text { page, html });
                    }
                    Ok(PreparedDocument::Workbook(book)) => {
                        view.database = Some(book.database.clone());
                        view.prepared = Some(PreparedDocument::Workbook(book));
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
    }
    fn show_word(&mut self, document: rich_documents::docx::WordDocument, cx: &mut Context<Self>) {
        self.word = Some(word::WordView::new(document, cx));
        self.layout_fallback = None;
        self.prepared = None;
        self.state = None;
        self.find = None;
        self._subscription = None;
    }

    fn show_layout(&mut self, path: rich_documents::DocumentPages, cx: &mut Context<Self>) {
        self.pdf = Some(cx.new(|cx| super::pdf::PdfReader::new(path.to_path_buf(), cx)));
        self.page_file = Some(path);
        self.reading_text = false;
        self.layout_fallback = None;
    }

    fn retry_layout(&mut self, cx: &mut Context<Self>) {
        if self.layout_retry.is_some() || self.layout_fallback != Some(LayoutFallback::MemoryInUse)
        {
            return;
        }
        let path = self.path.clone();
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| rich_documents::retry_layout(&path, cancel),
        );
        self.layout_retry = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.layout_retry = None;
                match result {
                    Ok(PreparedDocument::Pages(path)) => {
                        view.task = None;
                        view.show_layout(path, cx)
                    }
                    Ok(PreparedDocument::Word(document)) => {
                        view.task = None;
                        view.show_word(document, cx);
                    }
                    Ok(_) => unreachable!("layout retry returns formatted documents"),
                    Err(error) => {
                        log::warn!("Office layout retry failed: {error:#}");
                        view.layout_fallback = Some(LayoutFallback::from_error(&error));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn page(&mut self, start: u64, back: bool, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(PreparedDocument::Text { page, html }) = &self.prepared else {
            return;
        };
        let old_start = page.start;
        let html = *html;
        let source = page.source.clone();
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| source.read_cancellable(start, false, cancel),
        );
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok(page) => {
                        if back {
                            view.history.pop();
                        } else {
                            view.history.push(old_start);
                        }
                        view.set_text(&page.text, html, cx);
                        view.prepared = Some(PreparedDocument::Text { page, html });
                        view.error = None;
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
    }
    pub(super) fn show_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            let target = self
                .word
                .as_ref()
                .map(word::WordView::search_target)
                .or_else(|| self.state.clone().map(super::find::Target::Preview));
            if let Some(target) = target {
                let find = cx.new(|cx| super::find::FindBar::new(target, window, cx));
                if self.word.is_some() {
                    cx.observe(&find, |_, _, cx| cx.notify()).detach();
                }
                self.find = Some(find);
            }
        }
        if let Some(find) = &self.find {
            find.update(cx, |find, cx| find.show(false, window, cx));
        }
    }
}
impl DocumentReader {
    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(word) = &self.word {
            return v_flex()
                .size_full()
                .min_h_0()
                .children(self.find.clone())
                .child(div().flex_1().min_h_0().w_full().child(word.render(cx)))
                .into_any_element();
        }
        if let Some(pdf) = &self.pdf
            && !self.reading_text
        {
            return pdf.clone().into_any_element();
        }
        if self.table.is_none()
            && let Some(path) = self.database.clone()
        {
            self.table = Some(cx.new(|cx| super::sqlite::SqliteReader::new(path, window, cx)));
        }
        if let Some(table) = &self.table {
            return table.clone().into_any_element();
        }
        if let Some(state) = self.state.clone() {
            let back = self.history.last().copied();
            let next = match &self.prepared {
                Some(PreparedDocument::Text { page, .. }) if page.more => Some(page.end),
                _ => None,
            };
            let base = url::Url::from_file_path(&self.path).ok();
            let open = self.html_open.clone();
            let base_size = cx.theme().font_size;
            let font_size = crate::app::text_zoom::font_size(base_size, cx);
            let style = super::scaled_rich_text_style(
                TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
                    .with_foreground(super::editor_ink(cx)),
                f32::from(font_size) / f32::from(base_size),
                cx.theme().mono_font_size,
            );
            let text = TextView::new(&state)
                .text_size(font_size)
                .style(style)
                .scrollable(true)
                .size_full()
                .px_6()
                .py_4()
                .on_link_click(move |href, _, window, cx| {
                    if let (Some(base), Some(open)) = (&base, &open) {
                        super::markup::follow_link(base, open, href, window, cx);
                    }
                });
            let mut body = v_flex().size_full().min_h_0();
            if let Some(find) = &self.find {
                body = body.child(find.clone());
            }
            body = body.child(div().flex_1().min_h_0().child(text));
            if back.is_some() || next.is_some() {
                let mut controls = h_flex().gap_2().p_2();
                if let Some(start) = back {
                    controls = controls.child(
                        Button::new("document-previous")
                            .label(superfile_services::locale::tr("Previous page"))
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.page(start, true, cx)),
                            ),
                    );
                }
                if let Some(start) = next {
                    controls = controls.child(
                        Button::new("document-next")
                            .label(superfile_services::locale::tr("Next page"))
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.page(start, false, cx)),
                            ),
                    );
                }
                body = body.child(controls);
            }
            return body.into_any_element();
        }
        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                )
                .child(
                    Button::new("retry-document")
                        .label(superfile_services::locale::tr("Retry"))
                        .on_click(cx.listener(|view, _, _, cx| view.open(cx))),
                )
                .into_any_element();
        }
        super::preview_loading(cx)
    }
}
impl Render for DocumentReader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_content(window, cx);
        if let Some(reason) = self.layout_fallback {
            let word = self
                .path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"));
            let label = match (word, reason) {
                (true, LayoutFallback::MemoryInUse) => {
                    "Showing extracted text. Other previews are using the memory needed for Word formatting."
                }
                (true, LayoutFallback::PreviewLimit) => {
                    "Showing extracted text. This document exceeds the formatted preview limit."
                }
                (true, LayoutFallback::Unavailable) => {
                    "Showing extracted text. Word formatting could not be displayed."
                }
                (false, LayoutFallback::MemoryInUse) => {
                    "Showing Reading View. Other previews are using the memory needed for page layout."
                }
                (false, LayoutFallback::PreviewLimit) => {
                    "Showing Reading View. The document exceeds the size or complexity limit for page layout."
                }
                (false, LayoutFallback::Unavailable) => {
                    "Showing Reading View. The original page layout could not be rendered."
                }
            };
            let label = superfile_services::locale::tr(label);
            let mut notice = h_flex()
                .id("office-layout-notice")
                .role(Role::Status)
                .aria_label(label.clone())
                .gap_2()
                .px_3()
                .py_1()
                .bg(cx.theme().muted)
                .child(div().flex_1().text_sm().child(label));
            if reason == LayoutFallback::MemoryInUse {
                notice = notice.child(
                    Button::new("office-retry-layout")
                        .label(superfile_services::locale::tr(
                            match (word, self.layout_retry.is_some()) {
                                (true, true) => "Preparing formatted preview…",
                                (true, false) => "Retry formatted preview",
                                (false, true) => "Preparing page layout…",
                                (false, false) => "Retry page layout",
                            },
                        ))
                        .disabled(self.layout_retry.is_some())
                        .on_click(cx.listener(|view, _, _, cx| view.retry_layout(cx))),
                );
            }
            #[cfg(test)]
            let notice = gpui_kit::test::TestSupportExt::test_support(notice);
            return v_flex()
                .size_full()
                .min_h_0()
                .child(notice)
                .child(div().flex_1().min_h_0().w_full().child(content))
                .into_any_element();
        }
        if self.page_file.is_none() || !rich_documents::has_text_view(&self.path) {
            return content;
        }
        let label = if self.reading_text {
            "Original Layout"
        } else {
            "Reading View"
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex().child(
                    Button::new("office-layout-mode")
                        .label(superfile_services::locale::tr(label))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.reading_text = !view.reading_text;
                            view.task = None;
                            if let Some(pdf) = &view.pdf {
                                pdf.update(cx, |pdf, cx| pdf.set_active(!view.reading_text, cx));
                            }
                            if view.reading_text {
                                view.open(cx);
                            }
                            cx.notify();
                        })),
                ),
            )
            .child(div().flex_1().min_h_0().w_full().child(content))
            .into_any_element()
    }
}
impl super::contract::ReaderView for super::surface::SurfaceEntity<DocumentReader> {
    fn can_find(&self, cx: &App) -> bool {
        self.read(cx).can_find()
    }
    fn find(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |view, cx| view.show_find(window, cx));
    }
    fn kind(&self) -> &'static str {
        "document"
    }
    fn render(&mut self, _cx: &mut App) -> AnyElement {
        self.0.clone().into_any_element()
    }
    fn can_move_to_window(&self, cx: &App) -> bool {
        self.read(cx).can_move_to_window(cx)
    }
    fn set_active(&self, active: bool, cx: &mut App) {
        self.update(cx, |view, cx| view.set_active(active, cx));
    }
    fn operations(&self) -> super::contract::ReaderOperations<'_> {
        super::contract::ReaderOperations {
            playback: Some(self),
            ..Default::default()
        }
    }
}
impl super::surface::ReaderSurface for super::surface::SurfaceEntity<DocumentReader> {
    fn file_detail(&self, cx: &App) -> Option<String> {
        let view = self.read(cx);
        if let Some(pdf) = &view.pdf {
            let pages = pdf.read(cx).page_count();
            return (pages > 0).then(|| super::statistics::count_label("Pages", pages));
        }
        if let Some(PreparedDocument::Workbook(book)) = &view.prepared {
            return Some(super::statistics::count_label(
                "Worksheets",
                book.worksheets,
            ));
        }
        None
    }
    fn can_zoom_text(&self, cx: &App) -> bool {
        self.read(cx).can_find()
    }
}
impl super::contract::PlaybackState for super::surface::SurfaceEntity<DocumentReader> {
    fn playlist_shuffle(&self, cx: &App) -> Option<bool> {
        self.read(cx).playlist_shuffle()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cancellation, DocumentReader, LayoutFallback, PreparedDocument};
    use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

    #[gpui_kit::test]
    fn successful_retry_switches_to_pages_and_reuses_the_reading_view(cx: &mut TestAppContext) {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.odt");
        let mut file = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        file.start_file("content.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        file.write_all(br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p>Original text</text:p></office:text></office:body></office:document-content>"#).unwrap();
        drop(file.finish().unwrap());

        let PreparedDocument::Text { page, html } =
            superfile_services::rich_documents::read_document_text(&path, &Cancellation::default())
                .unwrap()
        else {
            panic!("expected manual text view")
        };
        cx.update(gpui_kit::init);
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| {
                    let mut view = DocumentReader::empty(path);
                    view.set_text(&page.text, html, cx);
                    view.prepared = Some(PreparedDocument::Text { page, html });
                    view.layout_fallback = Some(LayoutFallback::MemoryInUse);
                    view
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        let original = cx.update(|cx| reader.read(cx).state.as_ref().unwrap().entity_id());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("office-retry-layout", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = reader.read(cx);
            assert!(view.layout_fallback.is_none());
            assert!(view.page_file.is_some());
            assert!(!view.reading_text);
            window.click("office-layout-mode", cx);
            window.render_frame(cx);
            let view = reader.read(cx);
            assert!(view.reading_text && view.can_find());
            assert_eq!(view.state.as_ref().unwrap().entity_id(), original);
            assert!(window.try_find("office-layout-notice").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn automatic_layout_retry_keeps_text_and_manual_reading_has_no_warning(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.docx");
        let page = superfile_services::paged::text_value(
            "Retained reading text",
            &Cancellation::default(),
        )
        .unwrap();
        cx.update(gpui_kit::init);
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| {
                    let mut view = DocumentReader::empty(path);
                    view.set_text(&page.text, false, cx);
                    view.prepared = Some(PreparedDocument::Text { page, html: false });
                    view
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        let original = cx.update(|cx| reader.read(cx).state.as_ref().unwrap().entity_id());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("office-layout-notice").is_none());
            reader.update(cx, |view, cx| {
                view.layout_fallback = Some(LayoutFallback::MemoryInUse);
                cx.notify();
            });
            window.render_frame(cx);
            window.click("office-retry-layout", cx);
            assert!(reader.read(cx).can_find());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = reader.read(cx);
            let text = view.state.as_ref().unwrap();
            assert_eq!(text.entity_id(), original);
            assert_eq!(
                text.read(cx).rendered_text().source(),
                "Retained reading text"
            );
            assert_eq!(view.layout_fallback, Some(LayoutFallback::Unavailable));
            assert!(window.try_find("office-layout-notice").is_some());
            assert!(window.try_find("office-retry-layout").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn docx_find_reveals_matches_in_the_continuous_reader(cx: &mut TestAppContext) {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("continuous.docx");
        let mut file = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        file.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        file.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#).unwrap();
        for index in 0..80 {
            write!(file, "<w:p><w:r><w:t>Paragraph {index}</w:t></w:r></w:p>").unwrap();
        }
        file.write_all(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Last paragraph (查找)</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#.as_bytes()).unwrap();
        drop(file.finish().unwrap());

        cx.update(gpui_kit::init);
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| DocumentReader::new(path, false, false, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = reader.read(cx);
            assert!(view.word.is_some() && view.can_find());
            assert!(view.pdf.is_none());
            assert!(window.try_find("office-layout-mode").is_none());
            let super::super::find::Target::Group(group) =
                view.word.as_ref().unwrap().search_target()
            else {
                panic!("expected document-wide find");
            };
            assert_eq!(group.list.item_count(), 81);
            assert_eq!(group.list.logical_scroll_top().item_ix, 0);
            reader.update(cx, |view, cx| view.show_find(window, cx));
            window.render_frame(cx);
            window.input("Last paragraph (查找)", cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(140));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = reader.read(cx);
            let super::super::find::Target::Group(group) =
                view.word.as_ref().unwrap().search_target()
            else {
                panic!("expected document-wide find");
            };
            assert!(group.list.logical_scroll_top().item_ix > 0);
        })
        .unwrap();
    }
}
