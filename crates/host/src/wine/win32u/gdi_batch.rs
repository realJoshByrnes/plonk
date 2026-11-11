//! GDI drawing: paths, regions, bitmaps/DIBs, brushes, blits, world transforms, palettes, fonts resources, printing stubs (win32u calls filled in systematically; see docs/WIN32U_COVERAGE.txt).
use super::*;

/// Per-area state (add fields here; `Extra` owns one of these).
#[derive(Default)]
pub struct State {}

impl WineProc {
    /// Ok(None) = not handled here (falls through to the next handler / the lenient stub).
    pub(super) fn gdi_batch_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        let _ = (name, a);
        Ok(None)
    }
}
