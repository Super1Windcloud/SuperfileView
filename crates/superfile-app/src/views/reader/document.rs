//! Documents rendered with the same GPUI components on every desktop platform.
use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    base::{TextView, TextViewState},
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
    table: Option<Entity<super::sqlite::SqliteReader>>,
    pdf: Option<Entity<super::pdf::PdfReader>>,
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
            table: None,
            pdf: None,
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
        self.state.is_some()
    }
    pub(super) fn can_move_to_window(&self, _cx: &App) -> bool {
        self.task.is_none()
    }
    pub(super) fn hide(&mut self) {
        self.task = None;
    }
    pub(super) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if let Some(pdf) = &self.pdf {
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
        if self.pdf.is_some() {
            return;
        }
        if self.task.is_some()
            || self.prepared.is_some()
            || self.state.is_some()
            || self.database.is_some()
        {
            return;
        }
        self.error = None;
        let path = self.path.clone();
        let (cancel, guard) = Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| rich_documents::prepare(&path, cancel),
        );
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok(prepared) => {
                        match &prepared {
                            PreparedDocument::Text { page, html } => {
                                view.set_text(&page.text, *html, cx)
                            }
                            PreparedDocument::Workbook(book) => {
                                view.database = Some(book.database.clone())
                            }
                        }
                        view.prepared = Some(prepared);
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
            if let Some(state) = self.state.clone() {
                self.find = Some(cx.new(|cx| {
                    super::find::FindBar::new(super::find::Target::Preview(state), window, cx)
                }));
            }
        }
        if let Some(find) = &self.find {
            find.update(cx, |find, cx| find.show(false, window, cx));
        }
    }
}
impl Render for DocumentReader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(pdf) = &self.pdf {
            return pdf.clone().into_any_element();
        }
        if self.table.is_none() {
            if let Some(path) = self.database.clone() {
                self.table = Some(cx.new(|cx| super::sqlite::SqliteReader::new(path, window, cx)));
            }
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
            let text = TextView::new(&state)
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
impl super::contract::ReaderView for super::surface::SurfaceEntity<DocumentReader> {
    fn can_find(&self, cx: &App) -> bool {
        self.read(cx).state.is_some()
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
impl super::surface::ReaderSurface for super::surface::SurfaceEntity<DocumentReader> {}
impl super::contract::PlaybackState for super::surface::SurfaceEntity<DocumentReader> {
    fn playlist_shuffle(&self, cx: &App) -> Option<bool> {
        self.read(cx).playlist_shuffle()
    }
}
