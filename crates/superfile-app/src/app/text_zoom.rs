use super::actions::{DecreaseTextSize, IncreaseTextSize, ResetTextSize};
use gpui_kit::{App, Global, KeyBinding, Pixels, Task, px};
use superfile_services::preferences::{
    MAX_READER_FONT_SIZE, MIN_READER_FONT_SIZE, clamp_reader_font_offset, save_reader_font_offset,
};

struct TextZoom {
    offset: i16,
    write: Option<Task<()>>,
    save: fn(i16) -> anyhow::Result<()>,
}
impl Global for TextZoom {}

pub(super) fn init(offset: i16, cx: &mut App) {
    install(offset, save_reader_font_offset, cx);
}

fn install(offset: i16, save: fn(i16) -> anyhow::Result<()>, cx: &mut App) {
    cx.set_global(TextZoom {
        offset: clamp_reader_font_offset(offset),
        write: None,
        save,
    });
    cx.on_action(|_: &IncreaseTextSize, cx| adjust(1, cx));
    cx.on_action(|_: &DecreaseTextSize, cx| adjust(-1, cx));
    cx.on_action(|_: &ResetTextSize, cx| set_offset(0, cx));
    cx.bind_keys([
        KeyBinding::new("secondary-=", IncreaseTextSize, None),
        KeyBinding::new("secondary-shift-=", IncreaseTextSize, None),
        KeyBinding::new("secondary-+", IncreaseTextSize, None),
        KeyBinding::new("secondary--", DecreaseTextSize, None),
        KeyBinding::new("secondary-0", ResetTextSize, None),
    ]);
}

fn offset(cx: &App) -> i16 {
    cx.try_global::<TextZoom>().map_or(0, |zoom| zoom.offset)
}

pub(crate) fn font_size(base: Pixels, cx: &App) -> Pixels {
    (base + px(f32::from(offset(cx)))).clamp(
        px(f32::from(MIN_READER_FONT_SIZE)),
        px(f32::from(MAX_READER_FONT_SIZE)),
    )
}

pub(crate) fn adjust(step: i16, cx: &mut App) {
    set_offset(offset(cx) + step, cx);
}

fn set_offset(requested: i16, cx: &mut App) {
    let requested = clamp_reader_font_offset(requested);
    if super::updates::busy(cx) || requested == offset(cx) || !cx.has_global::<TextZoom>() {
        return;
    }
    let zoom = cx.global_mut::<TextZoom>();
    zoom.offset = requested;
    let save = zoom.save;
    let previous = zoom.write.take();
    let write = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        // Skip intermediate sizes queued while a preference write was in flight.
        if cx.read_global::<TextZoom, _>(|zoom, _| zoom.offset) != requested {
            return;
        }
        if let Err(error) = cx
            .background_executor()
            .spawn(async move { save(requested) })
            .await
        {
            log::error!("Cannot remember reader text size: {error:#}");
        }
    });
    cx.global_mut::<TextZoom>().write = Some(write);
    cx.refresh_windows();
}

pub(super) fn take_write(cx: &mut App) -> Option<Task<()>> {
    cx.global_mut::<TextZoom>().write.take()
}

#[cfg(test)]
pub(crate) fn init_test(offset: i16, cx: &mut App) {
    install(offset, |_| Ok(()), cx);
}
