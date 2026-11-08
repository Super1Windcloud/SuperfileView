use super::actions::{
    About, AddFolder, CloseWorkspace, NewFile, NewWindow, OpenFile, OpenFolder, OpenRemote,
    OpenSettings, Quit, SaveFile, TogglePreview, ToggleSidebar, ToggleWordWrap,
};
use gpui_kit::{App, Menu, MenuItem};
use superfile_services::locale::tr;

pub(super) fn update(cx: &mut App) {
    cx.set_dock_menu(vec![MenuItem::action(tr("New Window"), NewWindow)]);
    cx.set_menus(vec![
        Menu {
            name: "Superfile".into(),
            disabled: false,
            items: vec![
                MenuItem::action(tr("About Superfile"), About),
                MenuItem::separator(),
                MenuItem::action(tr("Settings…"), OpenSettings),
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
                MenuItem::action(tr("Close Window"), CloseWorkspace),
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
            ],
        },
    ]);
}
