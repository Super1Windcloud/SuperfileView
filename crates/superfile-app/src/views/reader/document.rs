//! Documents rendered with the same GPUI components on every desktop platform.
mod word;

use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    base::{TextView, TextViewState, TextViewStyle},
    component::{ActiveTheme, button::Button, h_flex, v_flex},
    *,
};
use std::path::PathBuf;
use superfile_services::{
    rich_documents::{self, PreparedDocument},
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
    database: Option<PathBuf>,
    images_database: Option<PathBuf>,
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
            database: None,
            images_database: None,
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
        self.task.is_none()
    }
    pub(super) fn hide(&mut self) {
        self.task = None;
    }
    pub(super) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if let Some(word) = &self.word {
            if !active {
                word.suspend(cx);
            }
            return;
        }
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
                    Ok(PreparedDocument::Word(document)) => {
                        view.word = Some(word::WordView::new(document, cx));
                    }
                    Ok(PreparedDocument::Pages(path)) => {
                        view.pdf =
                            Some(cx.new(|cx| super::pdf::PdfReader::new(path.to_path_buf(), cx)));
                        view.page_file = Some(path);
                    }
                    Ok(PreparedDocument::Text { page, html }) => {
                        view.set_text(&page.text, html, cx);
                        view.prepared = Some(PreparedDocument::Text { page, html });
                    }
                    Ok(PreparedDocument::Workbook(book)) => {
                        view.database = Some(book.database.clone());
                        view.images_database = book.images_database.clone();
                        view.prepared = Some(PreparedDocument::Workbook(book));
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
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
                .map(|word| word.search_target(cx))
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
            let images_database = self.images_database.clone();
            self.table = Some(cx.new(|cx| {
                super::sqlite::SqliteReader::with_images(path, images_database, window, cx)
            }));
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
        if matches!(self.prepared, Some(PreparedDocument::Text { .. }))
            && self
                .path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"))
        {
            return v_flex()
                .size_full()
                .min_h_0()
                .child(
                    h_flex()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .text_sm()
                        .child(superfile_services::locale::tr(
                            "Word formatting could not be loaded. Showing extracted text.",
                        ))
                        .child(
                            Button::new("retry-word-layout")
                                .label(superfile_services::locale::tr("Retry"))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.prepared = None;
                                    view.state = None;
                                    view.find = None;
                                    view.history.clear();
                                    view._subscription = None;
                                    view.open(cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex_1().min_h_0().child(content))
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
