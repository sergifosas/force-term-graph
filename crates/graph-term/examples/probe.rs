//! Phase 0 capability probe. Run inside the terminal you want to support:
//!
//! ```text
//! cargo run -p graph-term --example probe
//! ```
//!
//! It checks environment, pixel-size detection, Kitty / iTerm2 image display
//! (asking you what you saw), XTWINOPS size replies, and mouse reporting.
//! A summary is written to `probe-report.txt` for pasting into an issue/chat.

use std::fmt::Write as _;
use std::io::{BufRead, Write, stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::{cursor, execute, terminal};
use graph_core::{Anchor, Color, Scene, Shape, Vec2};
use graph_render::{PngSpeed, Renderer};
use graph_term::Protocol;
use graph_term::size::{detect_size, from_ioctl, from_wezterm_cli};

fn main() -> anyhow::Result<()> {
    let mut report = String::new();
    macro_rules! log {
        ($($t:tt)*) => {{ let line = format!($($t)*); println!("{line}"); writeln!(report, "{line}").ok(); }};
    }

    log!("== force-term-graph probe ==");
    log!("os: {} / {}", std::env::consts::OS, std::env::consts::ARCH);
    for k in [
        "TERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "WEZTERM_PANE",
        "WEZTERM_EXECUTABLE",
        "KITTY_WINDOW_ID",
        "COLORTERM",
    ] {
        log!("env {k} = {:?}", std::env::var(k).ok());
    }
    log!("detected protocol: {:?}", Protocol::detect());

    log!("\n-- size --");
    log!("crossterm size (cells): {:?}", terminal::size().ok());
    log!("ioctl/window_size: {:?}", from_ioctl());
    log!("wezterm cli: {:?}", from_wezterm_cli());
    let size = detect_size();
    log!(
        "chosen: {size:?}  cell = {:.1}x{:.1}px",
        size.cell_width(),
        size.cell_height()
    );

    log!("\n-- images --");
    let mut renderer = Renderer::new()?;
    for proto in [Protocol::Iterm2, Protocol::Kitty] {
        let png = renderer.render_png(&test_scene(&proto.to_string()), PngSpeed::Fast)?;
        println!("\nShowing a 320x160 test image via {proto} below:\n");
        let mut out = stdout();
        let cols = (320.0 / size.cell_width()).ceil() as u16;
        let rows = (160.0 / size.cell_height()).ceil() as u16;
        out.write_all(&proto.encode(&png, cols, rows))?;
        // Leave room under the image (doNotMoveCursor / C=1).
        let rows = rows + 1;
        execute!(out, cursor::MoveToNextLine(rows))?;
        out.flush()?;
        let seen = ask(&format!(
            "Did you see a purple circle labelled '{proto}'? [y/n] "
        ));
        log!("{proto}: user saw image = {seen}");
        if proto == Protocol::Kitty {
            out.write_all(proto.clear())?;
            out.flush()?;
            let gone = ask("Did the kitty image disappear now? [y/n] ");
            log!("kitty delete works = {gone}");
        }
    }

    log!("\n-- XTWINOPS replies (CSI 14 t / CSI 16 t) --");
    terminal::enable_raw_mode()?;
    print!("\x1b[14t\x1b[16t");
    stdout().flush()?;
    let replies = collect_events(Duration::from_millis(800), |_| false);
    terminal::disable_raw_mode()?;
    log!(
        "events received after query: {}",
        if replies.is_empty() {
            "none".into()
        } else {
            replies.join(" ")
        }
    );

    log!("\n-- mouse --");
    println!("Move, click, drag and scroll the mouse for up to 8 seconds (press q to stop early).");
    terminal::enable_raw_mode()?;
    execute!(stdout(), event::EnableMouseCapture)?;
    let mouse = collect_events(
        Duration::from_secs(8),
        |e| matches!(e, Event::Key(k) if k.kind == KeyEventKind::Press && k.code == KeyCode::Char('q')),
    );
    execute!(stdout(), event::DisableMouseCapture)?;
    terminal::disable_raw_mode()?;
    let count = |pred: fn(&MouseEventKind) -> bool| {
        mouse
            .iter()
            .filter(|s| s.starts_with("mouse"))
            .filter(|s| {
                let kind = s.split('|').nth(1).unwrap_or("");
                pred(&parse_kind(kind))
            })
            .count()
    };
    log!(
        "mouse events: {} (moves {}, downs {}, drags {}, scrolls {})",
        mouse.iter().filter(|s| s.starts_with("mouse")).count(),
        count(|k| matches!(k, MouseEventKind::Moved)),
        count(|k| matches!(k, MouseEventKind::Down(_))),
        count(|k| matches!(k, MouseEventKind::Drag(_))),
        count(|k| matches!(k, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown))
    );
    for s in mouse.iter().take(6) {
        log!("  sample: {s}");
    }

    std::fs::write("probe-report.txt", &report)?;
    println!("\nReport written to probe-report.txt");
    Ok(())
}

fn ask(q: &str) -> bool {
    print!("{q}");
    stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    line.trim().eq_ignore_ascii_case("y")
}

/// Collects events for `dur` (or until `stop` returns true) as strings.
fn collect_events(dur: Duration, stop: impl Fn(&Event) -> bool) -> Vec<String> {
    let start = Instant::now();
    let mut out = Vec::new();
    while start.elapsed() < dur {
        if !event::poll(Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }
        let Ok(e) = event::read() else { break };
        if stop(&e) {
            break;
        }
        out.push(match &e {
            Event::Mouse(m) => format!("mouse|{:?}|col={} row={}", m.kind, m.column, m.row),
            Event::Key(k) => format!("key:{:?}", k.code),
            other => format!("{other:?}"),
        });
    }
    out
}

fn parse_kind(s: &str) -> MouseEventKind {
    use crossterm::event::MouseButton::Left;
    match s {
        _ if s.starts_with("Moved") => MouseEventKind::Moved,
        _ if s.starts_with("Down") => MouseEventKind::Down(Left),
        _ if s.starts_with("Drag") => MouseEventKind::Drag(Left),
        _ if s.starts_with("Up") => MouseEventKind::Up(Left),
        _ if s.starts_with("ScrollUp") => MouseEventKind::ScrollUp,
        _ if s.starts_with("ScrollDown") => MouseEventKind::ScrollDown,
        _ => MouseEventKind::ScrollLeft,
    }
}

fn test_scene(label: &str) -> Scene {
    Scene {
        width: 320,
        height: 160,
        background: Color::rgb(30, 30, 36),
        shapes: vec![
            Shape::Line {
                a: Vec2::new(60.0, 80.0),
                b: Vec2::new(260.0, 80.0),
                width: 2.0,
                color: Color::rgb(90, 90, 110),
            },
            Shape::Circle {
                center: Vec2::new(160.0, 70.0),
                radius: 34.0,
                fill: Color::rgb(166, 138, 249),
                stroke: Some((2.0, Color::rgb(255, 255, 255))),
            },
            Shape::Text {
                pos: Vec2::new(160.0, 112.0),
                text: label.into(),
                size: 22.0,
                color: Color::rgb(220, 221, 222),
                anchor: Anchor::TopCenter,
            },
        ],
    }
}
