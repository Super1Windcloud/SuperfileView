use super::{CONFIRMATION_PENDING, retain_write, take_write};
use gpui_kit::{AnyWindowHandle, App, Global, PromptButton, PromptLevel, component::WindowExt};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
};
use superfile_services::{
    file_associations::{self, ChangeResult, DefaultOffer},
    locale::tr,
};

#[derive(Default)]
struct Offers {
    pending: VecDeque<(PathBuf, AnyWindowHandle)>,
    seen: HashSet<String>,
    running: bool,
}
impl Global for Offers {}

pub(crate) fn enqueue(path: PathBuf, handle: AnyWindowHandle, cx: &mut App) {
    if !cx.has_global::<Offers>() {
        cx.set_global(Offers::default());
    }
    let offers = cx.global_mut::<Offers>();
    if !offers
        .pending
        .iter()
        .any(|(queued, window)| queued == &path && *window == handle)
    {
        offers.pending.push_back((path, handle));
    }
    if offers.running {
        return;
    }
    offers.running = true;
    cx.spawn(async move |cx| {
        while let Some((path, handle)) = cx.update(|cx| {
            let offers = cx.global_mut::<Offers>();
            let next = offers.pending.pop_front();
            offers.running = next.is_some();
            next
        }) {
            let offer = cx.background_executor()
                .spawn(async move { file_associations::default_offer(&path) })
                .await;
            let offer = match offer {
                Ok(Some(offer)) => offer,
                Ok(None) => continue,
                Err(error) => {
                    log::warn!("Cannot check eligibility for a default application prompt: {error:#}");
                    continue;
                }
            };
            let response = cx.update(|cx| {
                handle.update(cx, |_, window, cx| {
                    if window.has_active_dialog(cx)
                        || !cx.global_mut::<Offers>().seen.insert(offer.identifier().to_owned())
                    {
                        return None;
                    }
                    let title = tr("Make Superfile the default for this file type?");
                    let (message, accept) = if cfg!(windows) {
                        (
                            format!(".{}\n{}", offer.extension(), tr("Select Superfile for this file extension in Windows Settings.")),
                            tr("Open Default Apps"),
                        )
                    } else {
                        (format!(".{}", offer.extension()), tr("Set as Default"))
                    };
                    let buttons = [
                        PromptButton::ok(accept),
                        PromptButton::cancel(tr("Keep Current Default")),
                    ];
                    Some(window.prompt(PromptLevel::Info, &title, Some(&message), &buttons, cx))
                }).ok().flatten()
            });
            if let Some(response) = response {
                let confirmed = matches!(response.await, Ok(0));
                cx.update(|cx| complete(offer, confirmed, handle, cx));
            }
        }
    }).detach();
}

fn complete(offer: DefaultOffer, confirmed: bool, handle: AnyWindowHandle, cx: &mut App) {
    let previous = take_write(cx);
    let task = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        let change = cx
            .background_executor()
            .spawn(async move {
                if let Err(error) = offer.remember() {
                    log::warn!("Cannot remember the default application prompt: {error:#}");
                }
                confirmed.then(|| offer.apply())
            })
            .await;
        let Some(change) = change else { return };
        cx.update(|cx| {
            let message = match change {
                Ok(ChangeResult::SystemSettingsRequired) => {
                    cx.open_url("ms-settings:defaultapps");
                    return;
                }
                Ok(ChangeResult::Applied(_)) => "Default application updated.",
                Ok(ChangeResult::ConfirmationPending) => CONFIRMATION_PENDING,
                Err(error) => {
                    log::warn!("Cannot set Superfile as the default application: {error:#}");
                    "Could not change the default application."
                }
            };
            let _ = handle.update(cx, |_, window, cx| {
                window.push_notification(tr(message), cx)
            });
        });
    });
    retain_write(task, cx);
}
