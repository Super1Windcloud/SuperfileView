//! Serialized workspace URL delivery for the Superfile application protocol.
#![allow(unsafe_code)]
use gpui_kit::*;

#[derive(Default)]
struct OpenRequests(Option<Task<()>>);
impl Global for OpenRequests {}

pub fn init(cx: &mut App) {
    cx.set_global(OpenRequests::default());
}

pub fn receive(url: &url::Url, cx: &mut App) -> bool {
    if url.scheme() != "superfile" {
        return false;
    }
    if url.host_str() != Some("open-workspace") {
        return true;
    }
    let Some(path) = url.query_pairs().find_map(|(name, value)| {
        (name == "path").then(|| std::path::PathBuf::from(value.as_ref()))
    }) else {
        return true;
    };
    if !path.is_absolute() {
        return true;
    }
    let previous = cx.global_mut::<OpenRequests>().0.take();
    let task = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        let result = cx
            .background_executor()
            .spawn(async move {
                if !path.is_dir() || superfile_services::preview::is_iwork(&path) {
                    return None;
                }
                let recent = superfile_services::preferences::recent_workspaces()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|workspace| workspace.folders.as_slice() == [path.clone()]);
                Some((path, recent))
            })
            .await;
        if let Some((path, recent)) = result {
            cx.update(|cx| super::open_folder_workspace(path, recent, cx));
        }
    });
    cx.global_mut::<OpenRequests>().0 = Some(task);
    true
}
