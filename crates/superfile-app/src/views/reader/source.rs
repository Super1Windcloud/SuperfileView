use super::super::minimap::Minimap;
use crate::app::code_theme;
use crate::app::reader_jobs::{self, JobClass, Priority};
use gpui_kit::{
    component::{
        ActiveTheme,
        input::{Editor, EditorState, InputEvent},
    },
    *,
};
use std::rc::Rc;

/// Shared basic editor surface for text/code and read-only value dialogs.
/// Native gutter, folding and scrollbars share the editor's layout frame.
pub(super) struct SourceReader {
    pub edit: crate::app::edit_session::EditSession,
    pub minimap: Entity<Minimap>,
    pub find: Entity<super::find::FindBar>,
    font_size: Option<Pixels>,
    code_page: bool,
    _minimap_subscription: Subscription,
}
impl SourceReader {
    pub fn new(
        text: &str,
        language: &'static str,
        readonly: bool,
        scroll: ScrollHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(language)
                .auto_close(false)
                .searchable(false)
                .smart_indent(false)
                .line_number(!readonly)
                .folding(!readonly)
                .soft_wrap(false)
                .scroll_beyond_last_line(Some(0));
            if super::lexical::supports(language) {
                state.set_highlighter_factory(
                    Rc::new(|name| Some(Box::new(super::lexical::Lexical::new(name)))),
                    cx,
                );
            }
            state.set_value(text.to_owned(), window, cx);
            state.set_readonly(readonly, cx);
            state
        });
        let minimap = cx.new(|cx| Minimap::new(state.clone(), scroll, cx));
        let edit = crate::app::edit_session::EditSession::new(state.clone(), text, readonly, cx);
        let find = cx.new(|cx| {
            super::find::FindBar::new(super::find::Target::Editor(state.clone()), window, cx)
        });
        let map = minimap.clone();
        let subscription = cx.subscribe(&state, move |state, event, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).text().clone();
                map.update(cx, |map, cx| map.set_text(&text, cx));
            }
        });
        Self {
            edit,
            minimap,
            find,
            font_size: None,
            code_page: false,
            _minimap_subscription: subscription,
        }
    }
    pub fn is_dirty(&self) -> bool {
        self.edit.is_dirty()
    }
    pub fn mark_saved(&mut self, text: String, cx: &App) {
        self.edit.mark_saved(text, cx);
    }
    pub fn inherit(&mut self, old: &Self, cx: &mut App) {
        self.font_size = old.font_size;
        let offset = old.edit.editor().read(cx).scroll_offset();
        self.edit
            .editor()
            .update(cx, |s, cx| s.set_scroll_offset(offset, cx));
    }
    pub(super) fn use_code_theme_for_page(&mut self) {
        self.code_page = true;
    }

    pub fn render(&mut self, cx: &mut App) -> AnyElement {
        let font_size = crate::app::text_zoom::font_size(cx.theme().mono_font_size, cx);
        if let Some(previous) = self.font_size.replace(font_size)
            && font_size != previous
        {
            let ratio = f32::from(font_size) / f32::from(previous);
            self.edit.editor().update(cx, |editor, cx| {
                let offset = editor.scroll_offset();
                editor.set_scroll_offset(point(offset.x * ratio, offset.y * ratio), cx);
            });
        }
        // The toolkit invokes custom menus while updating EditorState. Capture
        // capabilities during rendering instead of re-entering that entity.
        let editor_state = self.edit.editor();
        let state = editor_state.read(cx);
        let capabilities = state.context_menu_capabilities();
        let menu_editable = capabilities.is_editable();
        let menu_copyable = capabilities.is_copyable();
        let palette = (self.code_page || state.language_name().as_ref() != "plain")
            .then(|| code_theme::active(cx))
            .flatten();
        let paper = palette
            .as_ref()
            .map_or_else(|| super::editor_paper(cx), |palette| palette.background());
        let editor = code_theme::editor(
            Editor::new(editor_state)
                .text_size(font_size)
                .context_menu(move |menu, _, _| {
                    super::editor_menu(menu, menu_editable, menu_copyable)
                })
                .readonly(self.edit.is_readonly() || self.edit.is_locked())
                .appearance(false)
                .bordered(false)
                .size_full(),
            editor_state,
            palette.clone(),
            cx,
        );
        self.minimap.update(cx, |map, cx| {
            map.set_preview(false, cx);
            map.set_code_colors(
                palette
                    .as_ref()
                    .map(|palette| (palette.background(), palette.foreground())),
                cx,
            );
        });
        let content = gpui_kit::component::h_flex()
            .size_full()
            .font_family("Menlo")
            .text_size(px(13.))
            .line_height(px(20.))
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(editor)
                    // Hide the duplicate vertical rail, not the horizontal scroll track.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .w(px(12.))
                            .bg(paper),
                    ),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_shrink_0()
                    .h_full()
                    .child(self.minimap.clone()),
            )
            .into_any_element();
        super::find::wrap(content, &self.find)
    }
}

struct TextDialog {
    source: Option<SourceReader>,
    page: Option<Entity<super::paged::PagedReader>>,
    task: Option<Task<()>>,
    error: Option<String>,
}
impl TextDialog {
    fn new(text: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if text.len() <= superfile_services::limits::TEXT_LINE_BYTES && !text.contains('\0') {
            return Self {
                source: Some(SourceReader::new(
                    &text,
                    "plain",
                    true,
                    ScrollHandle::new(),
                    window,
                    cx,
                )),
                page: None,
                task: None,
                error: None,
            };
        }
        let (cancel, guard) = superfile_services::work::Cancellation::task();
        let work = reader_jobs::run(
            cx.background_executor().clone(),
            JobClass::Projection,
            Priority::Interactive,
            cancel,
            move |cancel| superfile_services::paged::text_value(&text, cancel),
        );
        let task = cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.task = None;
                match result {
                    Ok(page) => {
                        view.page = Some(cx.new(|_| super::paged::PagedReader::text_only(page)))
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
        Self {
            source: None,
            page: None,
            task: Some(task),
            error: None,
        }
    }
}
impl Render for TextDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(source) = &mut self.source {
            return source.render(cx);
        }
        if let Some(page) = &self.page {
            return page.clone().into_any_element();
        }
        if let Some(error) = &self.error {
            return div().p_3().child(error.clone()).into_any_element();
        }
        super::preview_loading(cx)
    }
}
pub(super) fn show_text_dialog(title: String, text: String, window: &mut Window, cx: &mut App) {
    let content = cx.new(|cx| TextDialog::new(text, window, cx));
    crate::views::dialog_drag::open(window, cx, move |dialog, _, cx, drag| {
        dialog
            .title(crate::views::dialog_drag::title(drag, title.clone(), cx))
            .width(px(640.))
            .child(div().h(px(360.)).child(content.clone()))
    });
}

#[cfg(test)]
mod format_registry_tests {
    #[test]
    fn new_source_formats_have_compiled_grammars() {
        crate::app::languages::init();
        let registry = gpui_kit::component::highlighter::LanguageRegistry::singleton();
        for name in [
            "java", "sql", "make", "html", "c", "cpp", "python", "json", "ruby", "php", "csharp",
            "swift", "kotlin", "lua", "scala", "elixir", "zig", "graphql", "proto", "cmake",
            "astro", "svelte", "erb", "ejs", "jsdoc",
        ] {
            let grammar = registry
                .language(name)
                .expect("enabled grammar must be registered");
            assert!(grammar.language.is_some(), "{name}");
            assert!(!grammar.highlights.is_empty(), "{name}");
            let highlighter = gpui_kit::component::highlighter::SyntaxHighlighter::new(name);
            assert_eq!(
                highlighter.language().as_ref(),
                name,
                "invalid query for {name}"
            );
        }
    }
}
