//! Terminal size in cells *and pixels*. Pixels are the hard part, so we try
//! several strategies in order:
//!
//! 1. `crossterm::terminal::window_size()` (TIOCGWINSZ on Unix; usually
//!    unavailable on Windows/ConPTY).
//! 2. `wezterm cli list --format json`, matched against `$WEZTERM_PANE`. This
//!    works on Windows.
//! 3. An assumed 8×16 px cell.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
    pub px_width: u32,
    pub px_height: u32,
    pub source: SizeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeSource {
    Ioctl,
    WeztermCli,
    Assumed,
}

impl TermSize {
    pub fn cell_width(&self) -> f32 {
        self.px_width as f32 / self.cols.max(1) as f32
    }

    pub fn cell_height(&self) -> f32 {
        self.px_height as f32 / self.rows.max(1) as f32
    }
}

pub fn detect_size() -> TermSize {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    if let Some(s) = from_ioctl() {
        return s;
    }
    if let Some((px_w, px_h, c, r)) = from_wezterm_cli() {
        return TermSize {
            cols: c,
            rows: r,
            px_width: px_w,
            px_height: px_h,
            source: SizeSource::WeztermCli,
        };
    }
    TermSize {
        cols,
        rows,
        px_width: cols as u32 * 8,
        px_height: rows as u32 * 16,
        source: SizeSource::Assumed,
    }
}

pub fn from_ioctl() -> Option<TermSize> {
    let ws = crossterm::terminal::window_size().ok()?;
    (ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0).then_some(TermSize {
        cols: ws.columns,
        rows: ws.rows,
        px_width: ws.width as u32,
        px_height: ws.height as u32,
        source: SizeSource::Ioctl,
    })
}

/// Returns `(pixel_width, pixel_height, cols, rows)` of the current WezTerm
/// pane.
pub fn from_wezterm_cli() -> Option<(u32, u32, u16, u16)> {
    let pane: u64 = std::env::var("WEZTERM_PANE").ok()?.trim().parse().ok()?;
    let out = Command::new(wezterm_binary())
        .args(["cli", "list", "--format", "json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let panes: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let p = panes
        .as_array()?
        .iter()
        .find(|p| p["pane_id"].as_u64() == Some(pane))?;
    let size = &p["size"];
    let get = |k: &str| size[k].as_u64();
    let (w, h) = (get("pixel_width")?, get("pixel_height")?);
    let (c, r) = (get("cols")?, get("rows")?);
    (w > 0 && h > 0).then_some((w as u32, h as u32, c as u16, r as u16))
}

/// `wezterm` next to `$WEZTERM_EXECUTABLE` if possible, else from PATH.
pub fn wezterm_binary() -> std::path::PathBuf {
    if let Ok(exe) = std::env::var("WEZTERM_EXECUTABLE") {
        let exe = std::path::PathBuf::from(exe);
        if let Some(dir) = exe.parent() {
            let name = if cfg!(windows) {
                "wezterm.exe"
            } else {
                "wezterm"
            };
            let candidate = dir.join(name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    "wezterm".into()
}
