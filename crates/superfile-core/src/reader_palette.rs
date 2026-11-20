//! Opaque sRGB colors for rendering a document independently of its UI toolkit.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct ReaderPalette {
    pub paper: [u8; 3],
    pub ink: [u8; 3],
    pub surface: [u8; 3],
    pub accent: [u8; 3],
}

impl ReaderPalette {
    pub fn is_dark(self) -> bool {
        let [r, g, b] = self.paper.map(u32::from);
        299 * r + 587 * g + 114 * b < 128_000
    }

    pub fn hex([r, g, b]: [u8; 3]) -> String {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}
