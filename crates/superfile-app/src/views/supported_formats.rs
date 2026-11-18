use gpui_kit::{
    component::{
        ActiveTheme, Sizable, WindowExt, h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::ops::Range;
use superfile_services::{
    documents::format_help::{self, Editing, FormatHelp, Preview},
    locale::tr,
};

pub(crate) fn open(cx: &mut App) {
    cx.defer(|cx| {
        if let Some(handle) = cx.active_window().or_else(|| cx.windows().last().copied()) {
            let _ = handle.update(cx, |_, window, cx| {
                show(window, cx);
            });
        }
    });
}

fn show(window: &mut Window, cx: &mut App) -> Option<Entity<SupportedFormats>> {
    if window.has_active_dialog(cx) {
        return None;
    }
    let view = cx.new(|cx| SupportedFormats::new(window, cx));
    let content = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let size = window.viewport_size();
        dialog
            .title(tr("Supported File Formats"))
            .width((size.width - px(24.)).min(px(700.)))
            .child(
                div()
                    .h((size.height - px(116.)).clamp(px(180.), px(540.)))
                    .child(content.clone()),
            )
    });
    Some(view)
}

struct SupportedFormats {
    search: Entity<InputState>,
    entries: Vec<FormatHelp>,
    visible: Vec<usize>,
    scroll: UniformListScrollHandle,
    _search: Subscription,
}

impl SupportedFormats {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Search by extension or capability…"))
        });
        let subscription = cx.subscribe(&search, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.filter(cx);
            }
        });
        let entries = format_help::formats();
        Self {
            visible: (0..entries.len()).collect(),
            entries,
            search,
            scroll: UniformListScrollHandle::new(),
            _search: subscription,
        }
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let searchable = format!(
                    "{} {} {}",
                    entry.name,
                    preview(entry.preview),
                    editing(entry.editing)
                )
                .to_lowercase();
                query
                    .split_whitespace()
                    .all(|term| searchable.contains(term))
                    .then_some(i)
            })
            .collect();
        if !self.visible.is_empty() {
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        cx.notify();
    }
}

fn preview(value: Preview) -> String {
    match value {
        Preview::Text => tr("Text and source code"),
        Preview::Markdown => tr("Markdown rendering"),
        Preview::Html => tr("HTML rendering"),
        Preview::ConvertedMarkup => tr("Converted text or diagram"),
        Preview::Structured => tr("Structured data view"),
        Preview::Table => tr("Table preview"),
        Preview::Image => tr("Image preview"),
        Preview::Pdf => tr("PDF pages"),
        Preview::OfficeText => tr("Extracted document text"),
        Preview::Workbook => tr("Spreadsheet cells"),
        Preview::Book => tr("Book reader"),
        Preview::MindMap => tr("Mind map"),
        Preview::Outline => tr("Outline"),
        Preview::Diagram => tr("Diagram preview"),
        Preview::Vector => tr("Model or bitmap font preview"),
        Preview::Archive => tr("Archive contents and extraction"),
        Preview::Container => tr("Package contents"),
        Preview::Sqlite => tr("SQLite tables"),
        Preview::BinaryPlist => tr("Property list"),
        Preview::BinaryReport => tr("Metadata and data summary"),
        Preview::Executable => tr("Executable metadata"),
    }
}

fn editing(value: Editing) -> String {
    match value {
        Editing::Source => tr("Edit source"),
        Editing::ReadOnly => tr("Read only"),
        Editing::TextOnly => tr("Text files only"),
    }
}

fn row(name: String, preview: String, editing: String) -> Div {
    h_flex()
        .w_full()
        .h(px(34.))
        .items_center()
        .gap_2()
        .px_2()
        .child(div().w(relative(0.27)).min_w_0().truncate().child(name))
        .child(div().flex_1().min_w_0().truncate().child(preview))
        .child(div().w(relative(0.24)).min_w_0().truncate().child(editing))
}

impl Render for SupportedFormats {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.visible.len();
        v_flex()
            .id("supported-formats")
            .size_full()
            .min_h_0()
            .gap_2()
            .text_sm()
            .child(Input::new(&self.search).small().w_full())
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(tr("Source editing requires a writable text file within the editor limits. Unlisted UTF-8 files can open as plain text.")))
            .child(row(tr("File format"), tr("Preview"), tr("Editing"))
                .font_weight(FontWeight::MEDIUM)
                .border_b_1()
                .border_color(cx.theme().border))
            .when(count == 0, |column| {
                column.child(div().p_2().child(tr("No matching file types.")))
            })
            .child(uniform_list("supported-format-list", count, cx.processor(|view, range: Range<usize>, _, cx| {
                range.map(|index| {
                    let entry = &view.entries[view.visible[index]];
                    row(entry.name.clone(), preview(entry.preview), editing(entry.editing))
                        .border_b_1()
                        .border_color(cx.theme().border.opacity(0.5))
                }).collect()
            })).w_full().flex_1().min_h_0().track_scroll(&self.scroll))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(format!("{count} / {}", self.entries.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::{Editing, InputEvent, show};
    use gpui_kit::{
        AppContext, Bounds, Empty, TestAppContext, WindowBounds, WindowOptions,
        component::WindowExt, px, size, test::TestWindowExt,
    };

    #[gpui_kit::test]
    fn format_dialog_filters_extensions_and_shows_editable_capabilities(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(640.), px(540.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| Empty),
            )
            .unwrap()
            .0
        });
        let dialog = cx
            .update_window(handle, |_, window, cx| {
                let dialog = show(window, cx).unwrap();
                assert!(window.has_active_dialog(cx));
                window.render_frame(cx);
                dialog
            })
            .unwrap();
        let search = cx.update(|cx| dialog.read(cx).search.clone());
        for (query, expected) in [
            (".PDF", Some(Editing::ReadOnly)),
            (".md", Some(Editing::Source)),
            ("no-such-format", None),
        ] {
            cx.update_window(handle, |_, window, cx| {
                search.update(cx, |search, cx| {
                    search.set_value(query, window, cx);
                    cx.emit(InputEvent::Change);
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let state = dialog.read(cx);
                if let Some(expected) = expected {
                    assert!(state.visible.iter().any(|&i| {
                        let entry = &state.entries[i];
                        entry.name.eq_ignore_ascii_case(query) && entry.editing == expected
                    }));
                } else {
                    assert!(state.visible.is_empty());
                }
            });
        }
    }
}
