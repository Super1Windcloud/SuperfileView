use super::{OpenLink, diagrams::DiagramPreview, pictures::Pictures, source::SourceReader};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    base::{TextView, TextViewState, TextViewStyle},
    component::ActiveTheme,
    *,
};
use std::{cell::Cell, collections::HashMap, ops::Range, path::Path, rc::Rc};

pub(super) struct MarkupReader {
    pub source: SourceReader,
    pub(super) task_toggle: Option<super::ToggleTask>,
    pub preview: bool,
    pub html: bool,
    pub(super) state: Entity<TextViewState>,
    pub(super) find: Entity<super::find::FindBar>,
    pub scroll: ScrollHandle,
    pending_scroll: Option<ListOffset>,
    pending_scroll_source: Option<String>,
    base: url::Url,
    headings: HashMap<String, Range<usize>>,
    pending_anchor: Option<String>,
    preview_text: String,
    initial_preview: Option<String>,
    preview_generation: Rc<Cell<usize>>,
    projection_result: Rc<std::cell::RefCell<Option<String>>>,
    projection_task: Option<Task<()>>,
    format_path: Option<std::path::PathBuf>,
    _subscription: Subscription,
    reveal_pending: Rc<Cell<bool>>,
    pictures: Entity<Pictures>,
    _picture_subscription: Subscription,
    diagrams: Option<Entity<DiagramPreview>>,
    _diagram_subscription: Option<Subscription>,
}
impl MarkupReader {
    pub fn new(
        path: &Path,
        text: &str,
        format: superfile_services::documents::presentation::MarkupFormat,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let scroll = ScrollHandle::new();
        use superfile_services::documents::presentation::MarkupFormat;
        let html = format == MarkupFormat::Html
            || (format == MarkupFormat::Converted
                && superfile_services::text_formats::renders_html(path));
        let preview = cx
            .try_global::<crate::app::ReaderPreferences>()
            .is_none_or(|preferences| {
                if html {
                    preferences.html_preview
                } else {
                    preferences.markdown_preview
                }
            });
        let format_path = (format == MarkupFormat::Converted).then(|| path.to_path_buf());
        let preview_content = if format_path.is_some() { "" } else { text };
        let normalized = (!html)
            .then(|| superfile_services::markdown_preview::normalize_tables(preview_content));
        let preview_content = normalized.as_deref().unwrap_or(preview_content);
        let (marked, _) = superfile_services::markup_highlights::preview(preview_content, html);
        let preview_content = marked.as_ref();
        let state = cx.new(|cx| {
            if html {
                TextViewState::html("", cx)
            } else {
                TextViewState::markdown("", cx)
            }
        });
        let base = url::Url::from_file_path(path).expect("absolute document path");
        let pictures =
            cx.new(|cx| Pictures::new(preview_content.to_owned(), html, base.clone(), cx));
        let picture_subscription = cx.observe(&pictures, |_, cx| cx.refresh_windows());
        let reveal_pending = Rc::new(Cell::new(false));
        let subscription = cx.observe(&state, move |_, cx| {
            // Parsing completes asynchronously, including when no anchor is pending.
            // Refresh the reader so its initial loading state cannot remain blank.
            cx.refresh_windows();
        });
        let diagrams = (!html).then(|| cx.new(DiagramPreview::new));
        let diagram_subscription = diagrams.as_ref().map(|diagrams| {
            let state = state.downgrade();
            cx.observe(diagrams, move |_, cx| {
                let _ = state.update(cx, |state, cx| state.invalidate_inline_layout(cx));
                cx.refresh_windows();
            })
        });
        let find = cx.new(|cx| {
            super::find::FindBar::new(super::find::Target::Preview(state.clone()), window, cx)
        });
        let mut reader = Self {
            find,
            task_toggle: None,
            pictures,
            _picture_subscription: picture_subscription,
            diagrams,
            _diagram_subscription: diagram_subscription,
            _subscription: subscription,
            reveal_pending,
            source: SourceReader::new(
                text,
                superfile_core::language_for_path(path),
                false,
                scroll.clone(),
                window,
                cx,
            ),
            state,
            scroll,
            pending_scroll: None,
            pending_scroll_source: None,
            preview,
            html,
            base,
            headings: headings(preview_content),
            pending_anchor: None,
            preview_text: text.to_owned(),
            initial_preview: Some(preview_content.to_owned()),
            preview_generation: Rc::new(Cell::new(0)),
            projection_result: Rc::default(),
            projection_task: None,
            format_path,
        };
        if reader.format_path.is_some() {
            reader.project(text.to_owned(), cx);
        }
        reader
    }
    pub fn refresh_dependencies(&mut self, cx: &mut App) {
        self.pictures
            .update(cx, |pictures, cx| pictures.refresh_local_images(cx));
    }
    pub fn inherit_preview_scroll(&mut self, old: &Self, cx: &App) {
        self.pending_scroll = Some(old.state.read(cx).list_state().logical_scroll_top());
    }
    pub fn sync_preview(&mut self, cx: &mut App) {
        let text = self.source.edit.editor().read(cx).value().to_string();
        if text == self.preview_text {
            return;
        }
        self.preview_text.clone_from(&text);
        self.initial_preview = None;
        self.preview_generation
            .set(self.preview_generation.get().wrapping_add(1));
        if self.format_path.is_some() {
            self.project(text, cx);
            return;
        }
        let text = if self.html {
            text
        } else {
            superfile_services::markdown_preview::normalize_tables(&text).into_owned()
        };
        self.apply_text(text, cx);
    }
    fn project(&mut self, text: String, cx: &mut App) {
        let Some(path) = self.format_path.clone() else {
            return;
        };
        self.projection_result.borrow_mut().take();
        let slot = self.projection_result.clone();
        let html = self.html;
        let (cancel, guard) = superfile_services::work::Cancellation::task();
        use crate::app::reader_jobs::{self, JobClass, Priority};
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| {
                cancel.check()?;
                let rendered = superfile_services::text_formats::project(&path, &text);
                cancel.check()?;
                Ok(if html {
                    rendered
                } else {
                    superfile_services::markdown_preview::normalize_tables(&rendered).into_owned()
                })
            },
        );
        self.projection_task = Some(cx.spawn(async move |cx| {
            let _guard = guard;
            if let Ok(rendered) = work.await {
                *slot.borrow_mut() = Some(rendered);
                cx.update(|cx| cx.refresh_windows());
            }
        }));
    }
    fn apply_text(&mut self, text: String, cx: &mut App) {
        let text = match superfile_services::markup_highlights::preview(&text, self.html).0 {
            std::borrow::Cow::Borrowed(_) => text,
            std::borrow::Cow::Owned(marked) => marked,
        };
        self.pending_scroll
            .get_or_insert_with(|| self.state.read(cx).list_state().logical_scroll_top());
        self.pending_scroll_source = Some(text.clone());
        self.headings = headings(&text);
        self.state.update(cx, |s, cx| s.set_text(&text, cx));
        self.pictures.update(cx, |pictures, cx| {
            pictures.load(text.clone(), self.html, cx)
        });
        if let Some(diagrams) = &self.diagrams {
            diagrams.update(cx, |diagrams, cx| diagrams.reset(cx));
        }
    }
    pub fn toggle_task(
        &mut self,
        range: Range<usize>,
        checked: bool,
        revision: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self.html
            || self.format_path.is_some()
            || !self.preview
            || revision != self.preview_generation.get()
            || !self
                .source
                .edit
                .editor()
                .read(cx)
                .context_menu_capabilities()
                .is_editable()
        {
            return false;
        }
        let text = self.source.edit.editor().read(cx).value().to_string();
        if text != self.preview_text {
            self.sync_preview(cx);
            return false;
        }
        let (projected, omitted) =
            superfile_services::markdown_preview::normalize_tables_with_mapping(&text);
        let (projected, inserted) =
            superfile_services::markup_highlights::preview(&projected, false);
        if self.state.read(cx).rendered_text().source() != projected.as_ref() {
            return false;
        }
        let mut offset = range.start;
        for insert in inserted.iter().rev() {
            if insert.end <= offset {
                offset -= insert.len();
            } else if insert.contains(&offset) {
                return false;
            }
        }
        for gap in omitted {
            if gap.start <= offset {
                offset += gap.len();
            } else {
                break;
            }
        }
        let Some(marker) = task_marker(&text, offset) else {
            return false;
        };
        if !matches!(text.as_bytes()[marker], b' ' | b'x' | b'X')
            || (text.as_bytes()[marker] != b' ') == checked
        {
            return false;
        }
        let utf16 = text[..marker].encode_utf16().count();
        self.source.edit.editor().update(cx, |editor, cx| {
            let selection = editor.selected_range();
            let scroll = editor.scroll_offset();
            editor.unmark_text(window, cx);
            editor.replace_text_in_range(
                Some(utf16..utf16 + 1),
                if checked { "x" } else { " " },
                window,
                cx,
            );
            editor.unmark_text(window, cx);
            editor.set_selected_range(selection, cx);
            editor.set_scroll_offset(scroll, cx);
        });
        self.pending_scroll = Some(self.state.read(cx).list_state().logical_scroll_top());
        self.sync_preview(cx);
        true
    }

    pub fn reveal(&mut self, anchor: &str, _cx: &mut App) {
        self.pending_scroll = None;
        self.pending_anchor = Some(anchor.to_owned());
        self.reveal_pending.set(true);
    }
    pub fn set_images_active(&self, active: bool, cx: &mut App) {
        self.pictures
            .update(cx, |pictures, cx| pictures.set_active(active, cx));
        if let Some(diagrams) = &self.diagrams {
            diagrams.update(cx, |diagrams, cx| diagrams.set_active(active, cx));
        }
    }
    pub fn render(&mut self, open: OpenLink, cx: &mut App) -> AnyElement {
        self.set_images_active(self.preview, cx);
        if !self.preview {
            return self.source.render(cx);
        }
        let projected = self.projection_result.borrow_mut().take();
        if let Some(text) = projected {
            self.projection_task = None;
            if self.initial_preview.is_some() {
                self.headings = headings(&text);
                self.initial_preview = Some(text);
            } else {
                self.apply_text(text, cx);
            }
        }
        if let Some(text) = self.initial_preview.take() {
            let state = self.state.downgrade();
            let generation = self.preview_generation.clone();
            let revision = generation.get();
            // First configure the Markdown plugins in this frame, then enqueue
            // the text once. Do not parse once without plugins and again with them.
            cx.defer(move |cx| {
                if generation.get() == revision {
                    let _ = state.update(cx, |state, cx| state.set_text(&text, cx));
                }
            });
        }
        let preview_ready = self
            .pending_scroll_source
            .as_ref()
            .is_none_or(|expected| self.state.read(cx).rendered_text().source() == expected);
        if self
            .pending_anchor
            .as_ref()
            .is_some_and(|a| !self.headings.contains_key(a))
        {
            self.pending_anchor = None;
            self.reveal_pending.set(false);
        }
        if let Some(anchor) = &self.pending_anchor
            && preview_ready
        {
            let range = self.headings.get(anchor).and_then(|r| {
                self.state
                    .read(cx)
                    .rendered_text()
                    .range_for_source(r.clone())
            });
            if let Some(range) = range {
                self.state.update(cx, |s, cx| {
                    let _ = s.reveal_range(range, cx);
                });
                self.pending_anchor = None;
                self.reveal_pending.set(false);
            }
        }
        let list = self.state.read(cx).list_state().clone();
        if !self.state.read(cx).rendered_text().source().is_empty()
            && preview_ready
            && let Some(offset) = self.pending_scroll.take()
        {
            self.pending_scroll_source = None;
            let list = list.clone();
            let generation = self.preview_generation.clone();
            let revision = generation.get();
            // Apply after the newly parsed document has reset its block list.
            cx.defer(move |cx| {
                if generation.get() == revision && list.item_count() > 0 {
                    list.scroll_to(ListOffset {
                        item_ix: offset.item_ix.min(list.item_count() - 1),
                        offset_in_item: offset.offset_in_item,
                    });
                    cx.refresh_windows();
                }
            });
        }

        if preview_ready {
            self.pending_scroll_source = None;
        }
        self.source.minimap.update(cx, |map, cx| {
            map.set_preview_list(list.clone(), cx);
            map.set_preview(true, cx);
        });
        let image_base = self.base.clone();
        let pictures = self.pictures.read(cx).sources.clone();
        let requested_images = self.pictures.read(cx).requested.clone();
        let animations = self.pictures.read(cx).animations.clone();
        let stills = self.pictures.read(cx).stills.clone();
        let dark = super::reader_palette(cx).is_dark();
        let link_base = self.base.clone();
        let minimap = self.source.minimap.downgrade();
        list.set_scroll_handler(move |_, _, cx| {
            let _ = minimap.update(cx, |_, cx| cx.notify());
        });
        // The viewport owns a virtual block list; offscreen tables are not
        // rebuilt and laid out on every wheel event.
        let base_size = px(14.);
        let font_size = crate::app::text_zoom::font_size(base_size, cx);
        let style = super::scaled_rich_text_style(
            TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
                .with_foreground(super::editor_ink(cx)),
            f32::from(font_size) / f32::from(base_size),
            cx.theme().mono_font_size,
        );
        let mut table_style = style.table().clone();
        table_style.overflow.x = Some(Overflow::Scroll);
        let image_resolver: super::markup_extensions::ImageResolver =
            std::sync::Arc::new(move |uri: &SharedUri| {
                let selected = pictures
                    .get(uri.as_ref())
                    .and_then(|p| {
                        if dark {
                            p.dark.as_deref()
                        } else {
                            p.light.as_deref()
                        }
                    })
                    .unwrap_or(uri.as_ref());
                match image_base.join(selected) {
                    Ok(url)
                        if matches!(url.scheme(), "file" | "http" | "https")
                            && url.path().to_ascii_lowercase().ends_with(".gif") =>
                    {
                        // Stay pending until background parsing has registered
                        // the player; never start GPUI's whole-GIF loader.
                        let animation = animations.get(url.as_str()).cloned();
                        let requested = requested_images.clone();
                        let key = url.to_string();
                        ImageSource::from(move |_: &mut Window, cx: &mut App| {
                            requested.lock().unwrap().insert(key.clone());
                            animation.as_ref().and_then(|view| {
                                view.update(cx, |view, cx| {
                                    view.set_active(true, cx);
                                    view.image()
                                })
                            })
                        })
                    }
                    Ok(url) if matches!(url.scheme(), "file" | "http" | "https" | "data") => {
                        let image = stills.get(url.as_str()).cloned();
                        let requested = requested_images.clone();
                        let key = url.to_string();
                        ImageSource::from(move |_: &mut Window, cx: &mut App| {
                            requested.lock().unwrap().insert(key.clone());
                            image.as_ref().and_then(|image| {
                                image.update(cx, |image, cx| {
                                    image.set_active(true, cx);
                                    image.image()
                                })
                            })
                        })
                    }
                    Ok(url) => SharedUri::from(url.to_string()).into(),
                    Err(_) => SharedUri::from(selected.to_owned()).into(),
                }
            });
        let text_view = TextView::new(&self.state)
            .text_size(font_size)
            .style(style.with_table(table_style))
            .scrollable(true)
            .size_full()
            // Keep document padding inside the viewport so the component's
            // absolute scrollbar stays at the reading area's outer edge.
            .px_6()
            .py_0()
            .pr(px(136.));
        let extensions = if let Some(diagrams) = &self.diagrams {
            super::diagrams::with_alerts(
                diagrams.update(cx, |diagrams, cx| diagrams.extensions(cx)),
                self.base.clone(),
                open.clone(),
            )
        } else {
            gpui_kit::base::MarkdownExtensions::default()
        };
        let extensions = if !self.html && self.format_path.is_none() {
            super::markup_extensions::with_tasks(
                extensions,
                image_resolver.clone(),
                self.task_toggle.clone(),
                self.source.edit.editor().entity_id(),
                self.preview_generation.get(),
            )
        } else {
            super::markup_extensions::with_images(extensions, image_resolver.clone())
        };
        let text_view = text_view.markdown_extensions(extensions);
        let body = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .id(("markdown-scroll", self.state.entity_id()))
                    .size_full()
                    .child(
                        text_view
                            .image_source({
                                let resolver = image_resolver.clone();
                                move |uri| resolver(uri)
                            })
                            .on_link_click(move |href, _, window, cx| {
                                follow_link(&link_base, &open, href, window, cx);
                            }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(12.))
                    .w(px(110.))
                    .child(self.source.minimap.clone()),
            )
            .when(
                !self.preview_text.trim().is_empty()
                    && self.state.read(cx).rendered_text().source().is_empty(),
                |view| {
                    view.child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(cx.theme().background)
                            .child(super::preview_loading(cx)),
                    )
                },
            );
        let content = super::pictures::PictureViewport {
            owner: self.pictures.clone(),
            content: gpui_kit::component::v_flex()
                .size_full()
                .min_h_0()
                .relative()
                .child(div().flex_1().min_h_0().w_full().child(body))
                .into_any_element(),
        }
        .into_any_element();
        super::find::wrap(content, &self.find)
    }
}
pub(super) fn follow_link(
    base: &url::Url,
    open: &OpenLink,
    href: &str,
    window: &mut Window,
    cx: &mut App,
) {
    if let Ok(url) = base.join(href) {
        superfile_services::telemetry::reader_feature(
            "markdown",
            "follow_link",
            [(
                "destination",
                superfile_services::telemetry::text(match url.scheme() {
                    "file" => "local",
                    "http" | "https" => "web",
                    "mailto" => "email",
                    _ => "other",
                }),
            )],
        );
        match url.scheme() {
            "file" => {
                if let Ok(path) = url.to_file_path() {
                    open(path, url.fragment().map(decode_fragment), window, cx);
                }
            }
            "http" | "https" | "mailto" => cx.open_url(url.as_str()),
            _ => {}
        }
    }
}
// The mdast list-item span starts at its bullet, so matching the marker here
// cannot toggle examples in code blocks or similarly worded tasks elsewhere.
fn task_marker(text: &str, start: usize) -> Option<usize> {
    let line = text.get(start..)?.split(['\r', '\n']).next()?;
    let bytes = line.as_bytes();
    let mut index = match bytes.first()? {
        b'-' | b'+' | b'*' => 1,
        b'0'..=b'9' => {
            let digits = bytes
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits > 9 || !matches!(bytes.get(digits)?, b'.' | b')') {
                return None;
            }
            digits + 1
        }
        _ => return None,
    };
    let space = index;
    while matches!(bytes.get(index), Some(b' ' | b'\t')) {
        index += 1;
    }
    if index == space
        || bytes.get(index) != Some(&b'[')
        || !matches!(bytes.get(index + 1), Some(b' ' | b'x' | b'X'))
        || bytes.get(index + 2) != Some(&b']')
        || bytes
            .get(index + 3)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    Some(start + index + 1)
}

fn decode_fragment(value: &str) -> String {
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < value.len() {
        if value.as_bytes()[i] == b'%'
            && let Some(code) = value.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(code, 16)
        {
            bytes.push(byte);
            i += 3;
        } else {
            bytes.push(value.as_bytes()[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
fn headings(text: &str) -> HashMap<String, Range<usize>> {
    let mut out = HashMap::new();
    let mut counts = HashMap::<String, usize>::new();
    let mut offset = 0;
    let mut fence = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
        }
        if !fence && trimmed.starts_with('#') {
            let title = trimmed
                .trim_start_matches('#')
                .trim()
                .trim_end_matches('#')
                .trim();
            let slug = title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace() || matches!(c, '-' | '_'))
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect::<String>();
            let count = counts.entry(slug.clone()).or_default();
            let key = if *count == 0 {
                slug
            } else {
                format!("{slug}-{count}")
            };
            *count += 1;
            out.insert(key, offset..offset + line.trim_end().len());
        }
        offset += line.len();
    }
    out
}
#[cfg(test)]
mod tests {
    use super::{decode_fragment, headings};
    #[test]
    fn anchors_handle_unicode_duplicate_titles_and_fences() {
        let text = "# 中文 标题\n# 中文 标题\n```\n# ignore\n```\n";
        let map = headings(text);
        assert_eq!(&text[map["中文-标题"].clone()], "# 中文 标题");
        assert!(map.contains_key("中文-标题-1"));
        assert!(!map.contains_key("ignore"));
        assert_eq!(decode_fragment("%E4%B8%AD%E6%96%87"), "中文");
    }
}
