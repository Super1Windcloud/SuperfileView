//! Local builds do not initialize any remote telemetry transport.
use gpui_kit::*;
use superfile_services::telemetry::Consent;
#[derive(Default)]
pub(crate) struct Telemetry {
    pub consent: Consent,
}
impl Global for Telemetry {}
pub(crate) fn initialize(_stored: anyhow::Result<Option<Consent>>, cx: &mut App) {
    cx.set_global(Telemetry::default());
}
pub(crate) fn shutdown(_cx: &mut App) -> Option<Task<()>> {
    None
}
pub(crate) fn observe_commands(_cx: &mut App) {}
