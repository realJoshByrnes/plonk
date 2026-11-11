//! clipboard, keyboard layouts, display/monitor/DPI enumeration, window stations/desktops, raw input, IME stubs, hotkeys (win32u calls filled in systematically; see docs/WIN32U_COVERAGE.txt).
use super::*;

/// Per-area state (add fields here; `Extra` owns one of these).
#[derive(Default)]
pub struct State {}

impl WineProc {
    /// Ok(None) = not handled here (falls through to the next handler / the lenient stub).
    pub(super) fn misc_batch_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        let _ = (name, a);
        Ok(None)
    }
}
