//! Drag a dialog by its title without patching the component library.
//!
//! The library positions the surface and rebuilds it every frame, so the offset
//! lives on an entity and is applied as a relative `left`/`top`. Move and release
//! listeners are registered from the canvas paint callback: `Window::on_mouse_event`
//! is only valid during paint, and the gesture has to keep running outside the title.

use gpui_kit::{
    component::{WindowExt, dialog::Dialog},
    *,
};
use std::{cell::Cell, rc::Rc};

const DRAG_THRESHOLD: Pixels = px(4.);
const DRAG_MARGIN: Pixels = px(8.);
const MIN_VISIBLE: Pixels = px(48.);

pub(crate) struct DialogDrag {
    offset: Point<Pixels>,
    /// Last painted handle bounds. Written from paint without notifying.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    session: Option<Session>,
}

struct Session {
    start: Point<Pixels>,
    origin_offset: Point<Pixels>,
    origin_bounds: Bounds<Pixels>,
    moved: bool,
}

impl Default for DialogDrag {
    fn default() -> Self {
        Self {
            offset: point(px(0.), px(0.)),
            bounds: Rc::new(Cell::new(Bounds::default())),
            session: None,
        }
    }
}

impl DialogDrag {
    pub(crate) fn offset(&self) -> Point<Pixels> {
        self.offset
    }

    fn begin(&mut self, position: Point<Pixels>) {
        self.session = Some(Session {
            start: position,
            origin_offset: self.offset,
            origin_bounds: self.bounds.get(),
            moved: false,
        });
    }

    /// Returns whether this event is part of a drag past [`DRAG_THRESHOLD`].
    /// The handle bounds are the ones captured at mouse-down, so a clamp cannot
    /// accumulate when the pointer keeps moving after the dialog has hit the edge.
    fn drag_to(&mut self, position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
        let (origin_offset, origin_bounds, delta) = {
            let Some(session) = self.session.as_mut() else {
                return false;
            };
            let delta = position - session.start;
            if !session.moved && delta.x.abs() < DRAG_THRESHOLD && delta.y.abs() < DRAG_THRESHOLD {
                return false;
            }
            session.moved = true;
            (session.origin_offset, session.origin_bounds, delta)
        };
        self.offset = clamped_offset(
            origin_offset + delta,
            origin_offset,
            origin_bounds,
            viewport,
        );
        true
    }

    /// Ends the gesture. `true` means the pointer moved far enough to drag,
    /// so the caller consumes the mouse-up and a title-bar button does not also click.
    fn end(&mut self) -> bool {
        self.session.take().is_some_and(|session| session.moved)
    }
}

fn clamped_offset(
    requested: Point<Pixels>,
    origin_offset: Point<Pixels>,
    origin_bounds: Bounds<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let size = origin_bounds.size;
    if size.width <= px(0.) || size.height <= px(0.) {
        return requested;
    }
    let visible_w = MIN_VISIBLE.min(size.width);
    let visible_h = MIN_VISIBLE.min(size.height);
    let min_x = DRAG_MARGIN + visible_w - size.width;
    let max_x = (viewport.width - DRAG_MARGIN - visible_w).max(min_x);
    let min_y = DRAG_MARGIN;
    let max_y = (viewport.height - DRAG_MARGIN - visible_h).max(min_y);
    let predicted = origin_bounds.origin + (requested - origin_offset);
    let clamped = predicted.clamp(&point(min_x, min_y), &point(max_x, max_y));
    origin_offset + (clamped - origin_bounds.origin)
}

pub(crate) fn title_id(drag: &Entity<DialogDrag>) -> ElementId {
    ("dialog-drag-title", drag.entity_id()).into()
}

pub(crate) fn open(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Dialog, &mut Window, &mut App, &Entity<DialogDrag>) -> Dialog + 'static,
) -> Entity<DialogDrag> {
    let drag = cx.new(|_| DialogDrag::default());
    open_with(drag.clone(), window, cx, build);
    drag
}

pub(crate) fn open_with(
    drag: Entity<DialogDrag>,
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Dialog, &mut Window, &mut App, &Entity<DialogDrag>) -> Dialog + 'static,
) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let offset = drag.read(cx).offset();
        build(dialog, window, cx, &drag)
            .left(offset.x)
            .top(offset.y)
    });
}

pub(crate) fn title(
    drag: &Entity<DialogDrag>,
    label: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    handle(title_id(drag), drag.clone(), label, cx)
}

pub(crate) fn handle(
    id: impl Into<ElementId>,
    drag: Entity<DialogDrag>,
    child: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let (bounds, dragging) = {
        let state = drag.read(cx);
        (
            state.bounds.clone(),
            state.session.as_ref().is_some_and(|session| session.moved),
        )
    };
    let pressed = drag.clone();
    let moved = drag.clone();
    let released = drag.clone();
    div()
        .id(id)
        .w_full()
        .relative()
        .flex_shrink_0()
        .cursor(if dragging {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::OpenHand
        })
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            pressed.update(cx, |drag, _| drag.begin(event.position));
        })
        .child(
            canvas(
                |_, _, _| (),
                move |painted, _, window, _cx| {
                    bounds.set(painted);
                    let tracking = moved.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if !phase.capture() || event.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        let viewport = window.viewport_size();
                        let handled =
                            tracking.update(cx, |drag, _| drag.drag_to(event.position, viewport));
                        if handled {
                            window.refresh();
                            cx.stop_propagation();
                        }
                    });
                    let tracking = released.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if !phase.capture() || event.button != MouseButton::Left {
                            return;
                        }
                        if tracking.update(cx, |drag, _| drag.end()) {
                            window.refresh();
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(child)
        .test_support()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        AppContext as _, Bounds, Empty, ParentElement, TestAppContext, WindowBounds, WindowOptions,
        point, px, size, test::TestWindowExt,
    };

    fn host(cx: &mut TestAppContext) -> gpui_kit::AnyWindowHandle {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1000.), px(800.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| Empty),
            )
            .unwrap()
            .0
        })
    }

    #[test]
    fn threshold_and_viewport_clamp() {
        let mut drag = super::DialogDrag::default();
        let viewport = size(px(800.), px(600.));
        drag.bounds.set(Bounds::new(
            point(px(100.), px(80.)),
            size(px(200.), px(24.)),
        ));
        assert!(!drag.drag_to(point(px(2.), px(0.)), viewport));
        drag.begin(point(px(0.), px(0.)));
        assert!(!drag.drag_to(point(px(2.), px(0.)), viewport));
        assert_eq!(drag.offset(), point(px(0.), px(0.)));
        assert!(!drag.end());

        drag.begin(point(px(0.), px(0.)));
        assert!(drag.drag_to(point(px(30.), px(10.)), viewport));
        assert_eq!(drag.offset(), point(px(30.), px(10.)));
        // Height is 24px, so the whole handle stays inside. Width keeps 48px.
        // max origin = (800 - 8 - 48, 600 - 8 - 24) = (744, 568)
        // offset = that origin minus the handle origin (100, 80).
        assert!(drag.drag_to(point(px(5000.), px(5000.)), viewport));
        assert_eq!(drag.offset(), point(px(644.), px(488.)));
        assert!(drag.drag_to(point(px(9000.), px(9000.)), viewport));
        assert_eq!(drag.offset(), point(px(644.), px(488.)));
        assert!(drag.drag_to(point(px(-10000.), px(-10000.)), viewport));
        assert_eq!(drag.offset(), point(px(-244.), px(-72.)));
        assert!(drag.end());
        assert!(!drag.drag_to(point(px(10.), px(10.)), viewport));

        let mut unmeasured = super::DialogDrag::default();
        unmeasured.bounds.set(Bounds::default());
        unmeasured.begin(point(px(0.), px(0.)));
        assert!(unmeasured.drag_to(point(px(100.), px(80.)), viewport));
        assert_eq!(unmeasured.offset(), point(px(100.), px(80.)));
    }

    #[gpui_kit::test]
    fn dragging_the_title_moves_only_that_dialog(cx: &mut TestAppContext) {
        let window = host(cx);
        cx.update_window(window, |_, window, cx| {
            let first = super::open(window, cx, |dialog, _, cx, drag| {
                dialog.title(super::title(drag, "First", cx)).child("Body")
            });
            let second = super::open(window, cx, |dialog, _, cx, drag| {
                dialog.title(super::title(drag, "Second", cx)).child("Body")
            });
            let first_id = super::title_id(&first);
            let second_id = super::title_id(&second);
            window.render_frame(cx);
            window.render_frame(cx);

            let first_origin = window.find(first_id.clone()).bounds().origin;
            let second_bounds = window.find(second_id.clone()).bounds();
            window.drag(
                second_bounds.center(),
                second_bounds.center() + point(px(2.), px(1.)),
                cx,
            );
            assert_eq!(
                window.find(second_id.clone()).bounds().origin,
                second_bounds.origin,
                "a movement below 4px must not drag"
            );

            let from = window.find(second_id.clone()).bounds().center();
            window.drag(from, from + point(px(80.), px(40.)), cx);
            window.render_frame(cx);
            let moved = window.find(second_id.clone()).bounds().origin - second_bounds.origin;
            assert!(
                (moved.x - px(80.)).abs() <= px(1.) && (moved.y - px(40.)).abs() <= px(1.),
                "title moved by {moved:?}"
            );
            assert_eq!(
                window.find(first_id).bounds().origin,
                first_origin,
                "the dialog behind keeps its own offset"
            );
            assert_eq!(second.read(cx).offset(), point(px(80.), px(40.)));

            let from = window.find(second_id.clone()).bounds().center();
            window.drag(from, from + point(px(5000.), px(5000.)), cx);
            window.render_frame(cx);
            let clamped = window.find(second_id).bounds();
            let viewport = window.viewport_size();
            assert!(
                clamped.top() < viewport.height - px(8.)
                    && clamped.bottom() > px(8.)
                    && clamped.left() < viewport.width - px(8.)
                    && clamped.right() > px(8.),
                "handle left the viewport: {clamped:?}"
            );
        })
        .unwrap();
    }
}
