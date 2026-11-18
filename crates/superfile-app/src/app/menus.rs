use super::actions::{
    About, AddFolder, CheckForUpdates, CloseWindow, DecreaseTextSize, DefaultApplications,
    IncreaseTextSize, KeyboardShortcuts, NewFile, NewWindow, OpenFile, OpenFolder, OpenRemote,
    OpenSettings, OpenThemes, Quit, ResetTextSize, SaveFile, SupportedFormats, TogglePreview,
    ToggleSidebar, ToggleWordWrap,
};
use gpui_kit::{App, Menu, MenuItem};
use superfile_services::locale::tr;

pub(super) fn update(cx: &mut App) {
    cx.set_dock_menu(vec![MenuItem::action(tr("New Window"), NewWindow)]);
    cx.set_menus(definitions());
}

// Native menus and the in-window menu must expose the same actions on every OS.
pub(crate) fn definitions() -> Vec<Menu> {
    vec![
        Menu {
            name: "Superfile".into(),
            disabled: false,
            items: vec![
                MenuItem::action(tr("About Superfile"), About),
                MenuItem::action(tr("Check for Updates…"), CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action(tr("Settings…"), OpenSettings),
                MenuItem::action(tr("Browse themes…"), OpenThemes),
                MenuItem::action(tr("Default File Types…"), DefaultApplications),
                MenuItem::action(tr("Keyboard Shortcuts…"), KeyboardShortcuts),
                MenuItem::action(tr("Supported File Formats"), SupportedFormats),
                MenuItem::separator(),
                MenuItem::action(tr("Quit Superfile"), Quit),
            ],
        },
        Menu {
            name: tr("File").into(),
            disabled: false,
            items: vec![
                MenuItem::action(tr("New File"), NewFile),
                MenuItem::action(tr("New Window"), NewWindow),
                MenuItem::separator(),
                MenuItem::action(tr("Open File…"), OpenFile),
                MenuItem::action(tr("Open Folder…"), OpenFolder),
                MenuItem::action(tr("Open Remote Workspace…"), OpenRemote),
                MenuItem::action(tr("Add Folder to Workspace…"), AddFolder),
                MenuItem::separator(),
                MenuItem::action(tr("Save"), SaveFile),
                MenuItem::separator(),
                MenuItem::action(tr("Close Window"), CloseWindow),
            ],
        },
        Menu {
            name: tr("View").into(),
            disabled: false,
            items: vec![
                MenuItem::action(tr("Toggle Sidebar"), ToggleSidebar),
                MenuItem::separator(),
                MenuItem::action(tr("Toggle Preview"), TogglePreview),
                MenuItem::action(tr("Word wrap"), ToggleWordWrap),
                MenuItem::separator(),
                MenuItem::action(tr("Increase text size"), IncreaseTextSize),
                MenuItem::action(tr("Decrease text size"), DecreaseTextSize),
                MenuItem::action(tr("Reset text size"), ResetTextSize),
            ],
        },
    ]
}
