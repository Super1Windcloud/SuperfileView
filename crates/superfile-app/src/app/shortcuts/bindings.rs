use crate::{
    app::actions::*,
    views::{self, explorer},
};
use gpui_kit::{
    component::{input, kbd::Kbd},
    *,
};
use std::collections::{BTreeMap, HashSet};
use superfile_services::{locale::tr, preferences};

pub(super) type Overrides = BTreeMap<String, String>;

pub(super) struct Command {
    pub(super) name: &'static str,
    pub(super) label: &'static str,
}

fn commands() -> Vec<Command> {
    let commands: &[(&dyn Action, &str)] = &[
        (&OpenSettings, "Open Settings"),
        (&NewWindow, "New window"),
        (&NewFile, "New file"),
        (&OpenFile, "Open file"),
        (&OpenFolder, "Open folder"),
        (&AddFolder, "Add workspace folder"),
        (&SaveFile, "Save file"),
        (&Refresh, "Refresh"),
        (&CloseWindow, "Close current tab; close window when empty"),
        (&CloseWorkspace, "Close workspace window"),
        (&Quit, "Quit Superfile"),
        (&SearchWorkspace, "Search workspace files"),
        (&ToggleSidebar, "Toggle sidebar"),
        (&TogglePreview, "Toggle preview / source"),
        (&ToggleWordWrap, "Toggle word wrap"),
        (&NextTab, "Next tab"),
        (&PreviousTab, "Previous tab"),
        (&NavigateBack, "Navigate back"),
        (&NavigateForward, "Navigate forward"),
        (&ToggleDarkMode, "Toggle theme"),
        (&KeyboardShortcuts, "Keyboard shortcuts"),
        (&input::Copy, "Copy"),
        (&input::Cut, "Cut"),
        (&input::Paste, "Paste"),
        (&input::SelectAll, "Select All"),
        (&input::Undo, "Undo (editor)"),
        (&input::Redo, "Redo (editor)"),
        (&input::Search, "Find in file"),
        (&views::NextMatch, "Next search result"),
        (&views::PreviousMatch, "Previous search result"),
        (&IncreaseTextSize, "Increase text size"),
        (&DecreaseTextSize, "Decrease text size"),
        (&ResetTextSize, "Reset text size"),
        (&explorer::Up, "Select previous file tree item"),
        (&explorer::Down, "Select next file tree item"),
        (&explorer::Expand, "Expand folder in file tree"),
        (&explorer::Collapse, "Collapse folder in file tree"),
        (&explorer::OpenSelected, "Open selected file tree item"),
    ];
    commands
        .iter()
        .map(|(action, label)| Command {
            name: action.name(),
            label,
        })
        .collect()
}

pub(super) struct Bindings {
    defaults: Vec<KeyBinding>,
    pub(super) commands: Vec<Command>,
    pub(super) overrides: Overrides,
    pub(super) saving: bool,
    write: Option<Task<()>>,
    save: fn(&Overrides) -> anyhow::Result<()>,
}
impl Global for Bindings {}

impl Bindings {
    pub(super) fn install(
        mut overrides: Overrides,
        save: fn(&Overrides) -> anyhow::Result<()>,
        cx: &mut App,
    ) {
        let defaults = cx
            .key_bindings()
            .borrow()
            .bindings()
            .cloned()
            .collect::<Vec<_>>();
        let mut commands = commands();
        commands.retain(|command| {
            defaults
                .iter()
                .any(|binding| binding.action().name() == command.name)
        });
        overrides.retain(|name, value| {
            let Some(key) = parse_custom_key(value) else {
                return false;
            };
            *value = key.unparse();
            commands.iter().any(|command| command.name == name)
        });
        let mut state = Self {
            defaults,
            commands,
            overrides,
            saving: false,
            write: None,
            save,
        };
        while let Err((name, _)) = state.validate(&state.overrides) {
            state.overrides.remove(&name);
        }
        state.apply(cx);
        cx.set_global(state);
    }

    pub(super) fn label(&self, name: &str) -> &'static str {
        self.commands
            .iter()
            .find(|command| command.name == name)
            .map_or("Shortcut", |command| command.label)
    }

    pub(super) fn display(&self, name: &str) -> String {
        if let Some(custom) = self.overrides.get(name) {
            return format_key(custom);
        }
        let mut seen = HashSet::new();
        self.defaults
            .iter()
            .filter(|binding| binding.action().name() == name)
            .map(format_binding)
            .filter(|label| seen.insert(label.clone()))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    fn validate(&self, overrides: &Overrides) -> Result<(), (String, &'static str)> {
        for (name, value) in overrides {
            let key = Keystroke::parse(value).expect("validated shortcut");
            if let Some(other) = overrides.iter().find_map(|(other, value)| {
                (other != name && Keystroke::parse(value).ok().as_ref() == Some(&key))
                    .then_some(self.label(other))
            }) {
                return Err((name.clone(), other));
            }
            if let Some(other) = self.defaults.iter().find(|binding| {
                let action = binding.action().name();
                action != name
                    && !overrides.contains_key(action)
                    && binding
                        .match_keystrokes(std::slice::from_ref(&key))
                        .is_some()
            }) {
                return Err((name.clone(), self.label(other.action().name())));
            }
        }
        Ok(())
    }

    fn apply(&self, cx: &mut App) {
        let mut seen = HashSet::new();
        let bindings = self
            .defaults
            .iter()
            .filter_map(|binding| {
                let action = binding.action();
                let Some(key) = self.overrides.get(action.name()) else {
                    return Some(binding.clone());
                };
                let predicate = binding.predicate();
                if !seen.insert((action.name(), predicate.clone())) {
                    return None;
                }
                Some(
                    KeyBinding::load(
                        key,
                        action.boxed_clone(),
                        predicate,
                        false,
                        None,
                        cx.keyboard_mapper().as_ref(),
                    )
                    .expect("validated shortcut"),
                )
            })
            .collect::<Vec<_>>();
        cx.clear_key_bindings();
        cx.bind_keys(bindings);
    }

    pub(super) fn save_and_apply(
        overrides: Overrides,
        view: WeakEntity<super::Shortcuts>,
        cx: &mut App,
    ) -> Result<(), String> {
        let state = cx.global_mut::<Self>();
        if state.saving || overrides == state.overrides {
            return Ok(());
        }
        if let Err((name, mut conflict)) = state.validate(&overrides) {
            if state.overrides.get(&name) == overrides.get(&name) {
                conflict = state.label(&name);
            }
            let error = if conflict == "Shortcut" {
                tr("This shortcut is already used by another control.")
            } else {
                tr("This shortcut is already used by {operation}.")
                    .replace("{operation}", &tr(conflict))
            };
            return Err(error);
        }
        let save = state.save;
        state.saving = true;
        let task = cx.spawn(async move |cx| {
            let result = cx
                .background_executor()
                .spawn(async move { save(&overrides).map(|()| overrides) })
                .await;
            cx.update(|cx| {
                cx.update_global::<Self, _>(|state, cx| {
                    state.saving = false;
                    match result {
                        Ok(overrides) => {
                            state.overrides = overrides;
                            state.apply(cx);
                            super::super::menus::update(cx);
                        }
                        Err(error) => {
                            log::error!("Cannot save keyboard shortcuts: {error:#}");
                            let _ = view.update(cx, |view, cx| {
                                view.error = Some(tr("Could not save keyboard shortcuts."));
                                cx.notify();
                            });
                        }
                    }
                });
                cx.refresh_windows();
            });
        });
        cx.global_mut::<Self>().write = Some(task);
        cx.refresh_windows();
        Ok(())
    }
}

pub(in crate::app) fn install(overrides: BTreeMap<String, String>, cx: &mut App) {
    Bindings::install(overrides, preferences::save_shortcut_overrides, cx);
}

pub(in crate::app) fn take_write(cx: &mut App) -> Option<Task<()>> {
    cx.global_mut::<Bindings>().write.take()
}

pub(super) fn parse_custom_key(source: &str) -> Option<Keystroke> {
    if source.is_empty() || source.len() > 128 || source.contains(char::is_whitespace) {
        return None;
    }
    let key = Keystroke::parse(source).ok()?;
    if key.key_char.is_some()
        || matches!(
            key.key.as_str(),
            "" | "control" | "ctrl" | "alt" | "option" | "shift" | "cmd" | "platform" | "function"
        )
    {
        return None;
    }
    let modifier = key.modifiers.control || key.modifiers.alt || key.modifiers.platform;
    let function_key = key
        .key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=35).contains(&number));
    (modifier || function_key).then_some(key)
}

fn format_key(source: &str) -> String {
    Keystroke::parse(source).map_or_else(|_| String::new(), |key| Kbd::format(&key))
}

fn format_binding(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|key| Kbd::format(key.as_keystroke()))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn format_shortcut_for_display(action: &dyn Action, fallback: &str, cx: &App) -> String {
    cx.key_bindings()
        .borrow()
        .bindings_for_action(action)
        .next_back()
        .map_or_else(|| format_key(fallback), format_binding)
}
