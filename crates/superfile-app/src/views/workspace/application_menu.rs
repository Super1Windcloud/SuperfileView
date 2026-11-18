use super::Workspace;
use gpui_kit::{
    assets::IconName,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    },
    *,
};
use superfile_services::locale::tr;

impl Workspace {
    pub(super) fn application_menu(&self, _: &mut Context<Self>) -> AnyElement {
        let focus = self.focus.clone();
        Button::new("application-menu")
            .ghost()
            .small()
            .w(px(28.))
            .h(px(28.))
            .flex_shrink_0()
            .icon(IconName::Menu)
            .tooltip(tr("Application Menu"))
            .accessibility_label(tr("Application Menu"))
            .dropdown_menu(move |mut menu, window, cx| {
                menu = menu.action_context(focus.clone()).min_w(px(240.));
                let mut menus = crate::app::menus::definitions().into_iter();
                if let Some(application) = menus.next() {
                    for group in menus {
                        menu = append_item(
                            menu,
                            &OwnedMenuItem::Submenu(group.owned()),
                            &focus,
                            window,
                            cx,
                        );
                    }
                    menu = menu.separator();
                    for item in application.owned().items {
                        menu = append_item(menu, &item, &focus, window, cx);
                    }
                }
                menu
            })
            .into_any_element()
    }
}

fn append_item(
    menu: PopupMenu,
    item: &OwnedMenuItem,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    match item {
        OwnedMenuItem::Separator => menu.separator(),
        OwnedMenuItem::Action {
            name,
            action,
            checked,
            disabled,
            ..
        } => menu.item(
            PopupMenuItem::new(name.clone())
                .action(action.boxed_clone())
                .checked(*checked)
                .disabled(*disabled),
        ),
        OwnedMenuItem::Submenu(group) => {
            let group = group.clone();
            let focus = focus.clone();
            menu.submenu(
                group.name.clone(),
                window,
                cx,
                move |mut menu, window, cx| {
                    menu = menu.action_context(focus.clone());
                    for item in &group.items {
                        menu = append_item(menu, item, &focus, window, cx);
                    }
                    menu
                },
            )
        }
        // OS-managed menus have no portable action list (e.g. macOS Services).
        OwnedMenuItem::SystemMenu(_) => menu,
    }
}
