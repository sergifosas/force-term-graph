//! Terminal image protocols. Each encoder turns a PNG into the escape
//! sequence that displays it at the current cursor position.

use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Kitty graphics protocol (images replaced in place by id). Native in
    /// kitty; in WezTerm it works but is slower than iTerm2 for large frames.
    Kitty,
    /// iTerm2 inline images (OSC 1337). The fastest path in WezTerm,
    /// including on Windows.
    Iterm2,
}

/// Image id reused for every frame so new frames replace the old one.
const KITTY_IMAGE_ID: u32 = 7_331;
const KITTY_CHUNK: usize = 4096;

/// Query sent to check Kitty support: a 1×1 RGB test image (`a=q` = check
/// only, nothing is stored), followed by a Primary Device Attributes request
/// that every terminal answers — its reply marks the end of the responses.
pub const KITTY_QUERY: &str = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";

impl Protocol {
    /// The best protocol for a recognised terminal (kitty → Kitty, WezTerm
    /// and iTerm2 → iTerm2). `None` for unknown terminals, which the pane
    /// frontend then asks about Kitty support with [`KITTY_QUERY`].
    pub fn detect() -> Option<Protocol> {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        if !var("KITTY_WINDOW_ID").is_empty() || var("TERM") == "xterm-kitty" {
            return Some(Protocol::Kitty);
        }
        match var("TERM_PROGRAM").as_str() {
            "WezTerm" | "iTerm.app" => Some(Protocol::Iterm2),
            _ if !var("WEZTERM_PANE").is_empty() => Some(Protocol::Iterm2),
            _ => None,
        }
    }

    /// Whether a reply collected after sending [`KITTY_QUERY`] says the
    /// terminal accepted the Kitty test image.
    pub fn kitty_reply_ok(reply: &str) -> bool {
        reply.contains("i=31;OK")
    }

    /// Escape sequence that draws `png` scaled to fill `cols`×`rows` cells
    /// at the cursor, without moving the cursor.
    pub fn encode(self, png: &[u8], cols: u16, rows: u16) -> Vec<u8> {
        let b64 = B64.encode(png);
        match self {
            Protocol::Kitty => encode_kitty(&b64, cols, rows),
            Protocol::Iterm2 => format!(
                "\x1b]1337;File=inline=1;size={};width={cols};height={rows};\
                 preserveAspectRatio=0;doNotMoveCursor=1:{b64}\x07",
                png.len()
            )
            .into_bytes(),
        }
    }

    /// Sequence that removes what we drew (best effort).
    pub fn clear(self) -> &'static [u8] {
        match self {
            // Delete by image id, freeing its data too (uppercase I).
            Protocol::Kitty => b"\x1b_Ga=d,d=I,i=7331,q=2\x1b\\",
            // iTerm2 images live in cells; clearing the screen removes them.
            Protocol::Iterm2 => b"",
        }
    }
}

fn encode_kitty(b64: &str, cols: u16, rows: u16) -> Vec<u8> {
    let bytes = b64.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / KITTY_CHUNK * 16 + 64);
    let chunks: Vec<&[u8]> = bytes.chunks(KITTY_CHUNK).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i != last);
        if i == 0 {
            // a=T transmit+display, f=100 PNG, c/r scale to cells, q=2 no
            // replies (they would arrive as fake keystrokes), C=1 keep cursor.
            out.extend_from_slice(
                format!(
                    "\x1b_Ga=T,f=100,i={KITTY_IMAGE_ID},p=1,c={cols},r={rows},q=2,C=1,m={more};"
                )
                .as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Protocol::Kitty => "kitty",
            Protocol::Iterm2 => "iterm2",
        })
    }
}

impl FromStr for Protocol {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "kitty" => Ok(Protocol::Kitty),
            "iterm2" | "iterm" => Ok(Protocol::Iterm2),
            other => Err(format!(
                "unknown protocol `{other}` (expected kitty or iterm2)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_chunks_large_payloads() {
        let png = vec![0u8; 10_000]; // ~13.3k base64 → 4 chunks
        let s = String::from_utf8(Protocol::Kitty.encode(&png, 80, 24)).unwrap();
        assert_eq!(s.matches("\x1b_G").count(), 4);
        assert!(s.starts_with("\x1b_Ga=T,f=100,i=7331,p=1,c=80,r=24,q=2"));
        assert_eq!(s.matches("m=1;").count(), 3);
        assert!(s.contains("\x1b_Gm=0;"));
        assert!(s.ends_with("\x1b\\"));
    }

    #[test]
    fn iterm2_has_size_and_dimensions() {
        let s = String::from_utf8(Protocol::Iterm2.encode(b"abc", 80, 24)).unwrap();
        assert!(s.starts_with("\x1b]1337;File=inline=1;size=3;width=80;height=24;"));
        assert!(s.ends_with(":YWJj\x07"));
    }

    #[test]
    fn kitty_reply_parsing_tolerates_lost_escape_bytes() {
        // On Windows the reply arrives as key events and ESC may be dropped.
        assert!(Protocol::kitty_reply_ok("_Gi=31;OK\\[?62;4c"));
        assert!(!Protocol::kitty_reply_ok("[?62;4c"));
        assert!(!Protocol::kitty_reply_ok("_Gi=31;EINVAL:bad\\[?62c"));
    }
}
