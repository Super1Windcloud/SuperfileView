use super::{Reader, Renderer, paged::PagedReader};
use crate::app::reader_jobs::{self, JobClass, Priority};
#[cfg(test)]
use gpui_kit::test::TestSupportExt;
use gpui_kit::{
    component::{
        ActiveTheme, h_flex,
        input::{EditorState, InputEvent, Rope},
    },
    *,
};
use std::time::Duration;
use superfile_services::{
    locale::tr,
    text_statistics::{SurroundingText, TextRegion, TextStatistics},
    work::Cancellation,
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

#[derive(Default)]
pub(crate) struct FileStatistics {
    source: Option<Source>,
    status: Status,
    surrounding: Option<(TextRegion, SurroundingText)>,
    clean_file: Option<TextRegion>,
    subscriptions: Vec<Subscription>,
    revision: u64,
    task: Option<Task<()>>,
}

impl FileStatistics {
    #[cfg(test)]
    pub(crate) fn counts(&self) -> Option<TextStatistics> {
        match self.status {
            Status::Ready(counts) => Some(counts),
            _ => None,
        }
    }

    pub(crate) fn set_reader(&mut self, reader: Option<&Reader>, cx: &mut Context<Self>) {
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
        let (cancel, guard) = Cancellation::task();
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(140))
                    .await;
            }
            let result = reader_jobs::run(
                cx.background_executor().clone(),
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
        let label = match self.status {
            Status::Empty => return div().into_any_element(),
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
            .child(div().truncate().child(label));
        #[cfg(test)]
        let footer = footer.test_support();
        footer.into_any_element()
    }
}
