use gpui_kit::{
    component::menu::{PopupMenu, PopupMenuItem},
    *,
};
use std::{cell::Cell, rc::Rc};

type Selection = Rc<Cell<Option<Option<usize>>>>;
struct ActiveMenu {
    window: WindowId,
    position: Point<Pixels>,
    menu: Entity<PopupMenu>,
    selected: Selection,
    _dismiss: Subscription,
}
#[derive(Default)]
struct Menus(Option<ActiveMenu>);
impl Global for Menus {}

pub(super) struct ContextMenu {
    selected: Selection,
}
impl ContextMenu {
    pub fn new(
        items: &[(Option<String>, bool)],
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        if !cx.has_global::<Menus>() {
            cx.set_global(Menus::default());
        }
        if let Some(previous) = cx.global_mut::<Menus>().0.take() {
            previous.selected.set(Some(None));
        }
        let selected = Rc::new(Cell::new(None));
        let selection = selected.clone();
        let menu = PopupMenu::build(window, cx, |mut menu, _, _| {
            for (index, (title, enabled)) in items.iter().enumerate() {
                menu = match title {
                    Some(title) => {
                        let selection = selection.clone();
                        menu.item(
                            PopupMenuItem::new(title.clone())
                                .disabled(!enabled)
                                .on_click(move |_, _, _| selection.set(Some(Some(index)))),
                        )
                    }
                    None => menu.separator(),
                };
            }
            menu
        });
        let selection = selected.clone();
        let dismiss = cx.subscribe(&menu, move |_, _: &DismissEvent, cx| {
            if selection.get().is_none() {
                selection.set(Some(None));
            }
            cx.global_mut::<Menus>().0 = None;
            cx.refresh_windows();
        });
        menu.focus_handle(cx).focus(window, cx);
        cx.global_mut::<Menus>().0 = Some(ActiveMenu {
            window: window.window_handle().window_id(),
            position: window.mouse_position(),
            menu,
            selected: selected.clone(),
            _dismiss: dismiss,
        });
        cx.refresh_windows();
        Some(Self { selected })
    }
    pub async fn choose(self, cx: &mut AsyncWindowContext) -> Option<usize> {
        loop {
            if let Some(selected) = self.selected.get() {
                return selected;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;
            if cx.update(|_, _| ()).is_err() {
                return None;
            }
        }
    }
}
pub(super) fn render(window: &Window, cx: &App) -> AnyElement {
    let active = cx
        .try_global::<Menus>()
        .and_then(|menus| menus.0.as_ref())
        .filter(|menu| menu.window == window.window_handle().window_id());
    match active {
        Some(active) => deferred(
            anchored()
                .position(active.position)
                .snap_to_window_with_margin(px(8.))
                .child(active.menu.clone()),
        )
        .with_priority(10)
        .into_any_element(),
        None => div().into_any_element(),
    }
}
