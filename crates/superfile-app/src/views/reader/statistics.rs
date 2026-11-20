use super::{Reader, Renderer, paged::PagedReader};
use crate::app::document_session::DocumentVersion;
use crate::app::reader_jobs::{self, JobClass, Priority};
#[cfg(test)]
use gpui_kit::test::TestSupportExt;
use gpui_kit::{
    component::{
        ActiveTheme, h_flex,
        input::{EditorState, InputEvent, Rope},
        tooltip::Tooltip,
    },
    *,
};
use std::{sync::Arc, time::Duration};
use superfile_services::{
    locale::tr,
    text_statistics::{SurroundingText, TextRegion, TextStatistics},
    work::{Cancellation, scheduler::Scheduler},
};

#[derive(PartialEq, Eq)]
enum Source {
    Buffer(Entity<EditorState>),
    Page(Entity<PagedReader>, Entity<EditorState>),
}

impl Source {
    fn for_reader(reader: &Reader, cx: &App) -> Option<Self> {
        if let Renderer::Paged { view, .. } = &reader.renderer {
            let source = view.read(cx).source_editor.as_ref()?;
            return Some(Self::Page(view.clone(), source.edit.editor().clone()));
        }
        reader
            .document
            .edit
            .as_ref()
            .map(|edit| Self::Buffer(edit.editor().clone()))
    }

    fn editor(&self) -> &Entity<EditorState> {
        match self {
            Self::Buffer(editor) | Self::Page(_, editor) => editor,
        }
    }
}

#[derive(Default)]
enum Status {
    #[default]
    Empty,
    Counting,
    Ready(TextStatistics),
    Unavailable,
}

pub(super) fn count_label(key: &str, value: usize) -> String {
    format!("{} {value}", tr(key))
}

fn format_label(reader: &Reader, cx: &App) -> String {
    let path = &reader.document.path;
    if let Renderer::Surface(surface) = &reader.renderer
        && surface
            .operations()
            .playback
            .is_some_and(|v| v.playlist_shuffle(cx).is_some())
    {
        return tr("Playlist");
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    for suffix in [
        ".fb2.zip",
        ".vega.json",
        ".tar.gz",
        ".tar.xz",
        ".tar.bz2",
        ".tar.zst",
        ".tar.zstd",
        ".tar.lz4",
    ] {
        if name.ends_with(suffix) {
            return suffix[1..].to_ascii_uppercase();
        }
    }
    if let Some(extension) = path
        .extension()
        .and_then(|v| v.to_str())
        .filter(|ext| !ext.is_empty())
    {
        return extension.to_ascii_uppercase();
    }
    let language = superfile_core::language_for_path(path);
    if language != "plain" {
        return language.to_ascii_uppercase();
    }
    tr(if reader.document.edit.is_some() {
        "Plain Text"
    } else {
        "File"
    })
}

fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

pub(crate) struct FileStatistics {
    scheduler: Arc<Scheduler>,
    document: Option<DocumentVersion>,
    format: String,
    detail: Option<String>,
    bytes: Option<u64>,
    size_task: Option<Task<()>>,
    source: Option<Source>,
    status: Status,
    surrounding: Option<(TextRegion, SurroundingText)>,
    clean_file: Option<TextRegion>,
    subscriptions: Vec<Subscription>,
    revision: u64,
    task: Option<Task<()>>,
}

impl FileStatistics {
    pub(crate) fn new(scheduler: Arc<Scheduler>) -> Self {
        Self {
            scheduler,
            document: None,
            format: String::new(),
            detail: None,
            bytes: None,
            size_task: None,
            source: None,
            status: Status::Empty,
            surrounding: None,
            clean_file: None,
            subscriptions: Vec::new(),
            revision: 0,
            task: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn counts(&self) -> Option<TextStatistics> {
        match self.status {
            Status::Ready(counts) => Some(counts),
            _ => None,
        }
    }

    pub(crate) fn set_reader(&mut self, reader: Option<&Reader>, cx: &mut Context<Self>) {
        self.update_properties(reader, cx);
        let source = reader.and_then(|reader| Source::for_reader(reader, cx));
        if self.source == source {
            return;
        }
        self.task = None;
        self.revision = self.revision.wrapping_add(1);
        self.source = source;
        self.surrounding = None;
        self.clean_file = None;
        self.status = Status::Empty;
        self.subscriptions.clear();
        if let Some(source) = &self.source {
            self.subscriptions
                .push(cx.subscribe(source.editor(), |_, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        let view = cx.entity().downgrade();
                        cx.defer(move |cx| {
                            let _ = view.update(cx, |view, cx| view.refresh(true, cx));
                        });
                    }
                }));
            if let Source::Page(page, _) = source {
                self.subscriptions
                    .push(cx.observe(page, |view, _, cx| view.refresh(false, cx)));
            }
            self.refresh(false, cx);
        }
        cx.notify();
    }

    fn update_properties(&mut self, reader: Option<&Reader>, cx: &mut Context<Self>) {
        let version = reader.map(|r| r.document.version());
        let changed = self.document != version;
        if changed {
            self.size_task = None;
            self.document = version;
            self.bytes = None;
            cx.notify();
        }
        let Some(reader) = reader else { return };
        let document = &reader.document;
        let detail = reader
            .projection
            .as_ref()
            .and_then(|v| v.table_detail(cx))
            .or_else(|| match &reader.renderer {
                Renderer::Surface(view) => view.file_detail(cx),
                Renderer::Paged { view, .. } if view.read(cx).source_editor.is_none() => {
                    view.read(cx).byte_range_label()
                }
                _ => None,
            });
        let format = format_label(reader, cx);
        if self.detail != detail || self.format != format {
            self.detail = detail;
            self.format = format;
            cx.notify();
        }
        if document.remote.is_some() {
            let bytes = document
                .remote_version
                .as_ref()
                .and_then(|files| files.get(&document.path))
                .and_then(Option::as_ref)
                .and_then(|stamp| stamp.size);
            if self.bytes != bytes {
                self.bytes = bytes;
                cx.notify();
            }
        } else if changed && document.draft_directory.is_none() {
            let path = document.path.clone();
            let task = cx.background_executor().spawn(async move {
                let metadata = std::fs::metadata(path).ok()?;
                metadata.is_file().then_some(metadata.len())
            });
            self.size_task = Some(cx.spawn(async move |view, cx| {
                let bytes = task.await;
                let _ = view.update(cx, |view, cx| {
                    if view.document == version {
                        view.bytes = bytes;
                        cx.notify();
                    }
                });
            }));
        }
    }

    fn refresh(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let Some(source) = &self.source else { return };
        let paged = match source {
            Source::Buffer(_) => None,
            Source::Page(page, _) => {
                let page = page.read(cx);
                let Some(region) = page.text_region() else {
                    return;
                };
                let dirty = page
                    .source_editor
                    .as_ref()
                    .is_some_and(|source| source.is_dirty());
                if !dirty
                    && self
                        .clean_file
                        .as_ref()
                        .is_some_and(|previous| previous.same_file(&region))
                {
                    return;
                }
                self.clean_file = (!dirty).then(|| region.clone());
                Some(region)
            }
        };
        let cached = self
            .surrounding
            .as_ref()
            .and_then(|(region, stats)| (Some(region) == paged.as_ref()).then_some(*stats));
        let text: Rope = source.editor().read(cx).text().clone();
        if !matches!(self.status, Status::Ready(_)) {
            self.status = Status::Counting;
            cx.notify();
        }
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let scheduler = self.scheduler.clone();
        let (cancel, guard) = Cancellation::task();
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(140))
                    .await;
            }
            let result = reader_jobs::run_with_scheduler(
                cx.background_executor().clone(),
                scheduler,
                JobClass::Projection,
                Priority::Background,
                cancel,
                move |cancel| {
                    let surrounding = match (cached, &paged) {
                        (Some(stats), _) => stats,
                        (None, Some(region)) => region.count_surrounding(cancel)?,
                        (None, None) => SurroundingText::default(),
                    };
                    let statistics = surrounding.count(text.chunks(), cancel)?;
                    Ok((statistics, paged.map(|region| (region, surrounding))))
                },
            )
            .await;
            let _ = view.update(cx, |view, cx| {
                if view.revision != revision {
                    return;
                }
                view.task = None;
                view.status = match result {
                    Ok((statistics, surrounding)) => {
                        view.surrounding = surrounding;
                        Status::Ready(statistics)
                    }
                    Err(error) => {
                        log::debug!("Cannot count document text: {error:#}");
                        Status::Unavailable
                    }
                };
                cx.notify();
            });
        }));
    }
}

impl Render for FileStatistics {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.document.is_none() {
            return div().into_any_element();
        }
        let counts = match self.status {
            Status::Empty => String::new(),
            Status::Counting => tr("Counting text…"),
            Status::Unavailable => tr("Text statistics unavailable"),
            Status::Ready(statistics) => {
                let words = tr(if statistics.words == 1 {
                    "{count} word"
                } else {
                    "{count} words"
                })
                .replace("{count}", &statistics.words.to_string());
                let lines = tr(if statistics.lines == 1 {
                    "{count} line"
                } else {
                    "{count} lines"
                })
                .replace("{count}", &statistics.lines.to_string());
                format!("{words} · {lines}")
            }
        };
        let mut parts = vec![self.format.clone()];
        parts.extend(self.detail.clone());
        parts.extend(self.bytes.map(file_size));
        if !counts.is_empty() {
            parts.push(counts);
        }
        let label = parts.join(" · ");
        let tooltip = label.clone();
        let footer = h_flex()
            .id("file-statistics")
            .w_full()
            .h(px(23.))
            .flex_shrink_0()
            .justify_end()
            .items_center()
            .px_3()
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .role(Role::Status)
            .aria_label(label.clone())
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(div().min_w_0().truncate().child(label));
        #[cfg(test)]
        let footer = footer.test_support();
        footer.into_any_element()
    }
}
