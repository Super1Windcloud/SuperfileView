use super::Workspace;
use gpui_kit::*;

pub(super) struct HistorySwipe {
    started: std::time::Instant,
    x: f32,
    y: f32,
    offsets: Vec<Pixels>,
}
impl Workspace {
    pub(super) fn zoom_document_text(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.modifiers;
        if !(modifiers.secondary() || modifiers.control)
            || modifiers.shift
            || modifiers.alt
            || modifiers.function
            || event.touch_phase == TouchPhase::Cancelled
        {
            return false;
        }
        let steps = match event.delta {
            ScrollDelta::Lines(delta) if delta.y != 0. && delta.y.is_finite() => delta.y.signum(),
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / 40.,
            _ => return false,
        };
        if steps == 0. || !steps.is_finite() {
            return false;
        }
        let Some(reader) = self.active.and_then(|index| self.tabs.get_mut(index)) else {
            return false;
        };
        let previous = reader.document.text_scale;
        if !reader.zoom_text(1.12_f32.powf(steps.clamp(-3., 3.)), cx) {
            return false;
        }
        self.history_swipe = None;
        if reader.document.text_scale != previous {
            cx.notify();
        }
        true
    }

    pub(super) fn history_swipe(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        if event.modifiers != Modifiers::default() {
            self.history_swipe = None;
            return;
        }
        if event.touch_phase == TouchPhase::Started {
            self.history_swipe = self
                .active_reader()
                .map_or(Some(Vec::new()), |reader| reader.horizontal_offsets(cx))
                .map(|offsets| HistorySwipe {
                    started: std::time::Instant::now(),
                    x: 0.,
                    y: 0.,
                    offsets,
                });
        }
        if event.touch_phase == TouchPhase::Cancelled {
            self.history_swipe = None;
            return;
        }
        let Some(swipe) = &mut self.history_swipe else {
            return;
        };
        swipe.x += f32::from(delta.x);
        swipe.y += f32::from(delta.y).abs();
        if event.touch_phase != TouchPhase::Ended {
            return;
        }
        let swipe = self.history_swipe.take().unwrap();
        let offsets = self
            .active_reader()
            .map_or(Some(Vec::new()), |reader| reader.horizontal_offsets(cx));
        let stationary = offsets.is_some_and(|offsets| {
            offsets.len() == swipe.offsets.len()
                && offsets
                    .iter()
                    .zip(&swipe.offsets)
                    .all(|(a, b)| f32::from(*a - *b).abs() < 1.)
        });
        if stationary
            && swipe.started.elapsed().as_millis() <= 800
            && swipe.x.abs() >= 80.
            && swipe.x.abs() > swipe.y * 2.
        {
            self.navigate(swipe.x < 0., window, cx);
            cx.stop_propagation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Workspace;
    use crate::views::{reader::Reader, workspace::Page};
    use gpui_kit::{
        AppContext, Bounds, InputEvent, Modifiers, Pixels, PlatformInput, Point, ScrollDelta,
        ScrollWheelEvent, TestAppContext, TouchPhase, WindowBounds, WindowOptions, point, px, size,
        test::TestWindowExt,
    };
    use superfile_core::{Document, DocumentKind};

    fn wheel(position: Point<Pixels>, y: f32, modifiers: Modifiers) -> PlatformInput {
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Lines(point(0., y)),
            modifiers,
            touch_phase: TouchPhase::Moved,
        }
        .to_platform_input()
    }

    #[gpui_kit::test]
    fn modifier_wheel_changes_the_rendered_text_size_only_in_the_active_document(
        cx: &mut TestAppContext,
    ) {
        let text = "Readable document text\n".repeat(200);
        let primary = Modifiers {
            control: !cfg!(target_os = "macos"),
            platform: cfg!(target_os = "macos"),
            ..Modifiers::default()
        };
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::app::remote::init(Default::default(), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1000.), px(700.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::new(None, window, cx);
                        workspace.page = Page::Document;
                        workspace.active = Some(0);
                        for (path, text) in [
                            ("first.txt", text.clone()),
                            ("second.csv", "name,value\nexample,123\n".into()),
                        ] {
                            workspace.tabs.push(Reader::new(
                                Document {
                                    path: path.into(),
                                    kind: DocumentKind::Text,
                                    text,
                                },
                                window,
                                cx,
                            ));
                        }
                        workspace
                    })
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let editor = workspace.read(cx).tabs[0].editor().unwrap();
            let initial_height = editor.read(cx).line_height().unwrap();
            let viewport = window.find(("input", editor.entity_id())).bounds();
            window.dispatch_event(wheel(viewport.center(), -3., Modifiers::default()), cx);
            window.render_frame(cx);
            assert_eq!(editor.read(cx).line_height(), Some(initial_height));
            assert!(editor.read(cx).scroll_offset().y < px(0.));

            window.dispatch_event(wheel(point(px(1.), px(1.)), 3., primary), cx);
            assert_eq!(workspace.read(cx).tabs[0].document.text_scale, 1.);

            window.dispatch_event(wheel(viewport.center(), 3., primary), cx);
            window.render_frame(cx);
            let enlarged = editor.read(cx).line_height().unwrap();
            assert!(enlarged > initial_height);
            assert_eq!(workspace.read(cx).tabs[1].document.text_scale, 1.);
            assert_eq!(editor.read(cx).text().to_string(), text);
            assert!(!workspace.read(cx).tabs[0].is_dirty());

            window.dispatch_event(wheel(viewport.center(), -3., primary), cx);
            window.render_frame(cx);
            assert!(editor.read(cx).line_height().unwrap() < enlarged);
            workspace.update(cx, |view, cx| {
                view.active = Some(1);
                view.tabs[1].set_preview(true, cx);
                cx.notify();
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let cell = window.find(("csv-cell", 0usize)).bounds();
            window.dispatch_event(wheel(cell.center(), 3., primary), cx);
            window.render_frame(cx);
            let resized = window.find(("csv-cell", 0usize)).bounds();
            assert!(resized.size.height > cell.size.height);
            assert!(workspace.read(cx).tabs[1].document.text_scale > 1.);
        })
        .unwrap();
    }
}
