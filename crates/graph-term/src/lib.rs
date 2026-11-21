//! Terminal frontend. Everything WezTerm/terminal-specific lives here and
//! nowhere else: graphics protocols, pixel-size detection, the pane loop.

pub mod pane;
pub mod presenter;
pub mod protocol;
pub mod size;

pub use pane::{PaneOptions, Viewport, run_pane};
pub use presenter::FrameReport;
pub use protocol::Protocol;
pub use size::{TermSize, detect_size};
