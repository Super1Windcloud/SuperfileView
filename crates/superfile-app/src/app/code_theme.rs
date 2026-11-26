use gpui_kit::{
    base::input::{DiagnosticColors, HighlightStyleResolver, InputEditorStyle},
    component::{
        ActiveTheme,
        highlighter::HighlightTheme,
        input::{Editor, EditorState},
    },
    *,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};
use superfile_services::code_themes::{self, Appearance, Color, Library};

pub(crate) struct CodeThemes {
    library: Library,
    light: Option<Arc<Palette>>,
    dark: Option<Arc<Palette>>,
    write: Option<Task<()>>,
}
impl Global for CodeThemes {}

pub(super) fn init(library: Library, cx: &mut App) {
    let light = resolve(&library, Appearance::Light);
    let dark = resolve(&library, Appearance::Dark);
    cx.set_global(CodeThemes {
        library,
        light,
        dark,
        write: None,
    });
}

fn resolve(library: &Library, appearance: Appearance) -> Option<Arc<Palette>> {
    library
        .selection(appearance)
        .and_then(|id| library.theme(id))
        .map(|theme| Arc::new(Palette::new(theme)))
}

pub(crate) fn library(cx: &App) -> &Library {
    static DEFAULT: LazyLock<Library> = LazyLock::new(Library::default);
    cx.try_global::<CodeThemes>()
        .map_or(&*DEFAULT, |state| &state.library)
}

pub(crate) fn active(cx: &App) -> Option<Arc<Palette>> {
    let themes = cx.try_global::<CodeThemes>()?;
    if cx.theme().is_dark() {
        themes.dark.clone()
    } else {
        themes.light.clone()
    }
}

pub(crate) fn update(cx: &mut App, library: Library) {
    if !cx.has_global::<CodeThemes>() {
        init(Library::default(), cx);
    }
    let state = cx.global_mut::<CodeThemes>();
    state.light = resolve(&library, Appearance::Light);
    state.dark = resolve(&library, Appearance::Dark);
    state.library = library.clone();
    let previous = state.write.take();
    let write = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        let result = cx
            .background_executor()
            .spawn(async move { code_themes::save(&library) })
            .await;
        if let Err(error) = result {
            log::error!("Cannot remember code themes: {error:#}");
            cx.update(|cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, cx| {
                        drop(window.prompt(
                            PromptLevel::Warning,
                            &superfile_services::locale::tr("Could not save code themes."),
                            Some(&format!("{error:#}")),
                            &[superfile_services::locale::tr("OK").as_str()],
                            cx,
                        ));
                    });
                }
            });
        }
    });
    cx.global_mut::<CodeThemes>().write = Some(write);
    cx.refresh_windows();
}

pub(super) fn take_write(cx: &mut App) -> Option<Task<()>> {
    cx.global_mut::<CodeThemes>().write.take()
}

pub(crate) struct Palette {
    editor: BTreeMap<String, Hsla>,
    syntax: BTreeMap<String, Hsla>,
    fallback: Arc<HighlightTheme>,
}

impl Palette {
    pub(crate) fn new(theme: &code_themes::Theme) -> Self {
        let mut editor = BTreeMap::new();
        let mut syntax = BTreeMap::new();
        for (name, color) in &theme.colors {
            if let Some(token) = name.strip_prefix("syntax.") {
                syntax.insert(token.to_owned(), to_hsla(*color));
            } else {
                editor.insert(name.clone(), to_hsla(*color));
            }
        }
        let fallback = match theme.appearance {
            Appearance::Light => HighlightTheme::default_light(),
            Appearance::Dark => HighlightTheme::default_dark(),
        };
        Self {
            editor,
            syntax,
            fallback,
        }
    }

    pub(crate) fn background(&self) -> Hsla {
        self.editor
            .get("editor.background")
            .copied()
            .or(self.fallback.style.editor_background)
            .unwrap_or_else(|| {
                if self.fallback.appearance.is_dark() {
                    black()
                } else {
                    white()
                }
            })
    }

    pub(crate) fn foreground(&self) -> Hsla {
        self.editor
            .get("editor.foreground")
            .copied()
            .or(self.fallback.style.editor_foreground)
            .unwrap_or_else(|| {
                if self.fallback.appearance.is_dark() {
                    white()
                } else {
                    black()
                }
            })
    }

    pub(crate) fn color(&self, field: &str) -> Hsla {
        match field {
            "editor.background" => self.background(),
            "editor.foreground" => self.foreground(),
            "editor.selection" => self
                .editor
                .get(field)
                .copied()
                .unwrap_or(self.foreground().opacity(0.23)),
            "editor.active_line.background" => self
                .editor
                .get(field)
                .copied()
                .unwrap_or(self.foreground().opacity(0.07)),
            _ => self
                .editor
                .get(field)
                .copied()
                .or_else(|| {
                    field
                        .strip_prefix("syntax.")
                        .and_then(|token| self.style(token)?.color)
                })
                .unwrap_or_else(|| self.foreground()),
        }
    }

    fn style_for_editor(self: &Arc<Self>, cx: &App) -> InputEditorStyle {
        let theme = cx.theme();
        InputEditorStyle {
            foreground: self.foreground(),
            muted_foreground: self
                .editor
                .get("editor.line_number")
                .copied()
                .unwrap_or(self.foreground().opacity(0.6)),
            background: self.background(),
            border: theme.border,
            selection: self.color("editor.selection"),
            caret: self.color("editor.caret"),
            diagnostics: DiagnosticColors {
                error: theme.danger,
                warning: theme.warning,
                info: theme.info,
                hint: theme.cyan,
            },
            highlight_styles: self.clone(),
            editor_invisible: self.editor.get("editor.invisible").copied(),
            editor_active_line: Some(self.color("editor.active_line.background")),
            editor_gutter_background: Some(
                self.editor
                    .get("editor.gutter.background")
                    .copied()
                    .unwrap_or(self.background()),
            ),
            fold_icon_renderer: None,
        }
    }
}

impl HighlightStyleResolver for Palette {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        let mut token = name;
        loop {
            if let Some(&color) = self.syntax.get(token) {
                let style = self.fallback.style(name).unwrap_or_default();
                return Some(HighlightStyle {
                    color: Some(color),
                    ..style
                });
            }
            let Some((parent, _)) = token.rsplit_once('.') else {
                return self.fallback.style(name);
            };
            token = parent;
        }
    }
}

pub(crate) fn to_hsla(color: Color) -> Hsla {
    let [r, g, b, a] = color.0.map(|channel| f32::from(channel) / 255.);
    Rgba { r, g, b, a }.into()
}

pub(crate) fn from_hsla(color: Hsla) -> Color {
    let Rgba { r, g, b, a } = color.to_rgb();
    Color([r, g, b, a].map(|channel| (channel.clamp(0., 1.) * 255.).round() as u8))
}

pub(crate) fn editor(
    editor: Editor,
    state: &Entity<EditorState>,
    palette: Option<Arc<Palette>>,
    cx: &App,
) -> AnyElement {
    match palette {
        None => editor.into_any_element(),
        Some(palette) => {
            let style = palette.style_for_editor(cx);
            ThemedEditor {
                content: editor
                    .bg(style.background)
                    .text_color(style.foreground)
                    .into_any_element(),
                state: state.clone(),
                style,
            }
            .into_any_element()
        }
    }
}

struct ThemedEditor {
    content: AnyElement,
    state: Entity<EditorState>,
    style: InputEditorStyle,
}

impl IntoElement for ThemedEditor {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ThemedEditor {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // The component projects its default style during layout. Replace the
        // style on this editor before its text is shaped and painted.
        let layout = self.content.request_layout(window, cx);
        self.state
            .update(cx, |editor, _| editor.set_editor_style(self.style.clone()));
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}
