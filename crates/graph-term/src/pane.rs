//! Interactive pane frontend.
//!
//! Frame policy:
//! - nothing changed → no frame; when the layout is settled and there is no
//!   input, the loop blocks on input and uses no CPU;
//! - while the layout moves or the user interacts → frames at up to `fps`
//!   (60 by default), at `interactive_scale` resolution (full by default);
//! - if `interactive_scale` < 1, one full-resolution frame once things calm
//!   down.
//!
//! Rendering, encoding and writing happen on a separate presenter thread
//! (see [`crate::presenter`]); this loop only handles input, physics and
//! building scenes, so it never waits on the terminal.
//!
//! Mouse: hover highlights, drag a node to move it (pinned while held), drag
//! the background to pan, scroll to zoom. Keys: see [`HELP`].

use std::io::{Write, stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as CtButton,
    MouseEvent, MouseEventKind,
};
use crossterm::{cursor, execute, terminal};
use graph_core::{AppEvent, AppState, MouseButton, Vec2};
use graph_render::Renderer;

use crate::presenter::{FrameJob, FrameReport, Pipeline};
use crate::protocol::{KITTY_QUERY, Protocol};
use crate::size::{TermSize, detect_size};

pub const HELP: &str =
    "drag move · scroll zoom · arrows pan · f fit · r relayout · esc deselect · q quit";

/// How long after the last input we keep rendering cheap frames before
/// upgrading to a full-resolution one.
const SETTLE_DELAY: Duration = Duration::from_millis(180);
const KEY_PAN_PX: f32 = 60.0;

#[derive(Debug, Clone)]
pub struct PaneOptions {
    /// `None` = auto-detect (queries the terminal for Kitty support).
    pub protocol: Option<Protocol>,
    /// Frame-rate cap while animating.
    pub fps: f32,
    /// Resolution factor for frames while animating (0.1..=1).
    pub interactive_scale: f32,
    /// Cap on the full-resolution frame's longest side, in pixels.
    pub max_dimension: u32,
    /// Show fps and frame timings in the status line.
    pub stats: bool,
    /// Benchmark: keep the layout moving for this long, then exit.
    pub bench: Option<Duration>,
}

impl Default for PaneOptions {
    fn default() -> Self {
        Self {
            protocol: None,
            fps: 60.0,
            interactive_scale: 1.0,
            max_dimension: 4096,
            stats: false,
            bench: None,
        }
    }
}

/// Pixel geometry of the image area (the pane minus the status row).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub size: TermSize,
    pub image_rows: u16,
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn new(size: TermSize) -> Self {
        let image_rows = size.rows.saturating_sub(1).max(1);
        Self {
            size,
            image_rows,
            width: (size.cols as f32 * size.cell_width()).max(16.0),
            height: (image_rows as f32 * size.cell_height()).max(16.0),
        }
    }

    /// Centre of a cell, in image pixels (mouse reports are per cell).
    pub fn cell_to_px(&self, col: u16, row: u16) -> Vec2 {
        Vec2::new(
            (col as f32 + 0.5) * self.size.cell_width(),
            ((row as f32 + 0.5) * self.size.cell_height()).min(self.height),
        )
    }

    /// Render scale for a full-quality frame given the size cap.
    pub fn full_scale(&self, max_dimension: u32) -> f32 {
        (max_dimension as f32 / self.width.max(self.height)).min(1.0)
    }

    /// Half the larger cell side: how far a cell-quantised pointer can be
    /// from the real cursor position.
    pub fn hit_slop(&self) -> f32 {
        self.size.cell_width().max(self.size.cell_height()) * 0.5
    }
}

/// Restores the terminal even on early return or panic.
struct TerminalGuard {
    protocol: Option<Protocol>,
}

/// Windows timers tick every ~15.6 ms by default, which would quantise every
/// frame wait and cap the loop well below 60 fps. Ask for 1 ms while the
/// pane is open (restored on exit).
#[cfg(windows)]
mod timer {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
        fn timeEndPeriod(period: u32) -> u32;
    }
    pub fn begin() {
        // SAFETY: plain Win32 calls with a constant argument.
        unsafe { timeBeginPeriod(1) };
    }
    pub fn end() {
        // SAFETY: paired with `begin`.
        unsafe { timeEndPeriod(1) };
    }
}
#[cfg(not(windows))]
mod timer {
    pub fn begin() {}
    pub fn end() {}
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        timer::begin();
        terminal::enable_raw_mode()?;
        execute!(
            stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableMouseCapture
        )?;
        Ok(Self { protocol: None })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = stdout();
        if let Some(p) = self.protocol {
            let _ = out.write_all(p.clear());
        }
        let _ = execute!(
            out,
            event::DisableMouseCapture,
            terminal::Clear(terminal::ClearType::All),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        timer::end();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Quality {
    Low,
    Full,
}

#[derive(Debug)]
enum Action {
    None,
    Quit,
    Resize,
    App(AppEvent),
    /// Esc: deselect if something is selected, otherwise quit.
    Escape,
}

/// Runs the interactive pane until the user quits. Returns frame statistics
/// for the session.
pub fn run_pane(
    app: &mut AppState,
    renderer: &mut Renderer,
    opts: &PaneOptions,
) -> Result<FrameReport> {
    let mut guard = TerminalGuard::enter()?;
    let protocol = opts.protocol.unwrap_or_else(resolve_protocol);
    guard.protocol = Some(protocol);

    let pipeline = Pipeline::default();
    let stop_input = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|s| {
        let encoder = s.spawn(|| pipeline.run_encoder(renderer, protocol));
        let writer = s.spawn(|| pipeline.run_writer(opts.stats));
        let input = s.spawn(|| read_input(&tx, &stop_input));
        let result = event_loop(app, opts, protocol, &pipeline, &rx);
        pipeline.shutdown();
        stop_input.store(true, Ordering::Relaxed);
        for (name, handle) in [("encoder", encoder), ("writer", writer), ("input", input)] {
            handle
                .join()
                .map_err(|_| anyhow!("{name} thread panicked"))?;
        }
        result?;
        match pipeline.error() {
            Some(e) => Err(anyhow!("presenting frames failed: {e}")),
            None => Ok(pipeline.report()),
        }
    })
}

/// Input thread: forwards terminal events. Waiting for input here, instead
/// of with a timeout in the frame loop, matters on Windows: console waits
/// are coarse (~15.6 ms), which made frame timing irregular and capped the
/// measured rate at ~42 fps. The frame loop paces itself with precise sleeps.
fn read_input(tx: &mpsc::Sender<Event>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => match event::read() {
                Ok(ev) => {
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            },
            Ok(false) => {}
            Err(_) => return,
        }
    }
}

fn event_loop(
    app: &mut AppState,
    opts: &PaneOptions,
    protocol: Protocol,
    pipeline: &Pipeline,
    input: &mpsc::Receiver<Event>,
) -> Result<()> {
    let fps = opts.fps.clamp(1.0, 240.0);
    let frame_interval = Duration::from_secs_f32(1.0 / fps);
    let low_scale = opts.interactive_scale.clamp(0.1, 1.0);
    let started = Instant::now();

    let mut vp = apply_viewport(app, detect_size());
    let mut last_tick = Instant::now();
    let mut last_input = Instant::now();
    let mut next_frame = Instant::now();
    let mut shown: Option<Quality> = None;
    let mut need_frame = true;
    let mut clear = true;
    let mut events = Vec::new();

    loop {
        if pipeline.error().is_some() {
            return Ok(()); // run_pane reports it
        }
        if let Some(limit) = opts.bench {
            if started.elapsed() >= limit {
                return Ok(());
            }
            // Keep the layout moving so every frame is a real, new frame.
            if app.sim.alpha() < 0.5 {
                app.sim.reheat(1.0);
            }
        }

        let full = vp.full_scale(opts.max_dimension);
        let low = low_scale.min(full);
        let busy = need_frame
            || app.is_animating()
            || app.is_dragging()
            || (shown == Some(Quality::Low) && low < full);

        // Wait: until the next frame is due while things move (precise
        // sleep; input queues up meanwhile), or for input when idle.
        if busy {
            let wait = next_frame.saturating_duration_since(Instant::now());
            std::thread::sleep(wait.max(Duration::from_micros(500)));
        } else {
            match input.recv() {
                Ok(ev) => events.push(ev),
                Err(_) => return Err(anyhow!("terminal input closed")),
            }
        }
        events.extend(input.try_iter());

        if !events.is_empty() {
            let mut resized = false;
            // Coalesced: a burst of mouse moves becomes one frame.
            for ev in events.drain(..) {
                match action_for(ev, &vp) {
                    Action::Quit => return Ok(()),
                    Action::Escape if app.highlight.selected.is_some() => {
                        app.update(AppEvent::ClearSelection)
                    }
                    Action::Escape => return Ok(()),
                    Action::Resize => resized = true,
                    Action::App(ev) => app.update(ev),
                    Action::None => {}
                }
            }
            last_input = Instant::now();
            if resized {
                vp = apply_viewport(app, detect_size());
                clear = true;
                need_frame = true;
            }
        }

        let now = Instant::now();
        let dt = (now - last_tick).as_secs_f32().min(0.25);
        last_tick = now;
        if app.tick(dt) {
            need_frame = true;
        }

        let interacting =
            app.is_animating() || app.is_dragging() || last_input.elapsed() < SETTLE_DELAY;
        let frame_due = now >= next_frame;
        let upgrade = !interacting && shown == Some(Quality::Low) && low < full;

        if (need_frame && frame_due) || upgrade {
            let quality = if interacting && low < full {
                Quality::Low
            } else {
                Quality::Full
            };
            pipeline.submit(FrameJob {
                scene: app.scene(),
                scale: if quality == Quality::Low { low } else { full },
                cols: vp.size.cols,
                image_rows: vp.image_rows,
                status_row: vp.size.rows.saturating_sub(1),
                status: status_line(app, protocol),
                clear: std::mem::take(&mut clear),
            });
            // Fixed cadence; after a stall, restart it instead of bursting.
            next_frame += frame_interval;
            if next_frame <= now {
                next_frame = now + frame_interval;
            }
            shown = Some(quality);
            need_frame = false;
        }
    }
}

/// Picks the image protocol.
///
/// - Known terminals use their best protocol directly. In WezTerm that is
///   iTerm2: measured at 2544×1296 it sustains ~41 fps vs ~28 fps for Kitty.
/// - Unknown terminals are asked whether they support Kitty. On Windows the
///   reply arrives as synthetic key events (seen in the probe), so we consume
///   them here — up to the Device Attributes answer — before the main loop
///   could mistake them for keystrokes.
fn resolve_protocol() -> Protocol {
    if let Some(known) = Protocol::detect() {
        return known;
    }
    let guess = Protocol::Iterm2;
    let mut out = stdout();
    if out.write_all(KITTY_QUERY.as_bytes()).is_err() || out.flush().is_err() {
        return guess;
    }
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut reply = String::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if !event::poll(left).unwrap_or(false) {
            break;
        }
        if let Ok(Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            kind,
            ..
        })) = event::read()
            && kind != KeyEventKind::Release
        {
            reply.push(c);
        }
        // The DA1 reply `[?…c` terminates the answers.
        if reply.contains("[?") && reply.ends_with('c') {
            break;
        }
    }
    if Protocol::kitty_reply_ok(&reply) {
        Protocol::Kitty
    } else {
        guess
    }
}

fn apply_viewport(app: &mut AppState, size: TermSize) -> Viewport {
    let vp = Viewport::new(size);
    app.hit_slop = vp.hit_slop();
    if app.camera.viewport != Vec2::new(vp.width, vp.height) {
        app.update(AppEvent::Resize(vp.width, vp.height));
    }
    vp
}

fn action_for(ev: Event, vp: &Viewport) -> Action {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => key_action(k),
        Event::Mouse(m) => mouse_action(m, vp),
        Event::Resize(..) => Action::Resize,
        _ => Action::None,
    }
}

fn key_action(k: KeyEvent) -> Action {
    let app = Action::App;
    let pan = |x: f32, y: f32| Action::App(AppEvent::PanBy(Vec2::new(x, y)));
    match k.code {
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::Escape,
        KeyCode::Char('f') => app(AppEvent::FitView),
        KeyCode::Char('r') => app(AppEvent::Reheat),
        KeyCode::Char('+') | KeyCode::Char('=') => app(AppEvent::Zoom(2.0)),
        KeyCode::Char('-') => app(AppEvent::Zoom(-2.0)),
        KeyCode::Left | KeyCode::Char('h') => pan(KEY_PAN_PX, 0.0),
        KeyCode::Right | KeyCode::Char('l') => pan(-KEY_PAN_PX, 0.0),
        KeyCode::Up | KeyCode::Char('k') => pan(0.0, KEY_PAN_PX),
        KeyCode::Down | KeyCode::Char('j') => pan(0.0, -KEY_PAN_PX),
        _ => Action::None,
    }
}

fn mouse_action(m: MouseEvent, vp: &Viewport) -> Action {
    let p = vp.cell_to_px(m.column, m.row);
    let button = |b: CtButton| match b {
        CtButton::Left => MouseButton::Left,
        CtButton::Right => MouseButton::Right,
        CtButton::Middle => MouseButton::Middle,
    };
    Action::App(match m.kind {
        MouseEventKind::Moved | MouseEventKind::Drag(_) => AppEvent::PointerMove(p),
        MouseEventKind::Down(b) => AppEvent::PointerDown(p, button(b)),
        MouseEventKind::Up(b) => AppEvent::PointerUp(p, button(b)),
        MouseEventKind::ScrollUp => AppEvent::Scroll(p, 1.0),
        MouseEventKind::ScrollDown => AppEvent::Scroll(p, -1.0),
        _ => return Action::None,
    })
}

fn status_line(app: &AppState, protocol: Protocol) -> String {
    let left = match app.focused_node() {
        Some(id) => {
            let n = app.graph.node(id);
            let kind = if n.ghost { "unresolved" } else { &n.category };
            let degree = app.graph.degree(id);
            let plural = if degree == 1 { "" } else { "s" };
            format!(" ● {} · {kind} · {degree} link{plural}", n.label)
        }
        None => format!(
            " {} nodes · {} edges · {protocol}",
            app.graph.node_count(),
            app.graph.edge_count()
        ),
    };
    // The presenter truncates to the pane width.
    format!("{left}   {HELP}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::SizeSource;

    fn vp() -> Viewport {
        // The probe's real numbers: 318x55 cells, 2544x1320 px.
        Viewport::new(TermSize {
            cols: 318,
            rows: 55,
            px_width: 2544,
            px_height: 1320,
            source: SizeSource::WeztermCli,
        })
    }

    #[test]
    fn viewport_reserves_status_row_and_maps_cells() {
        let v = vp();
        assert_eq!(v.image_rows, 54);
        assert_eq!((v.width, v.height), (2544.0, 1296.0));
        assert_eq!(v.cell_to_px(0, 0), Vec2::new(4.0, 12.0));
        assert_eq!(v.cell_to_px(10, 2), Vec2::new(84.0, 60.0));
        // The status row clamps into the image.
        assert_eq!(v.cell_to_px(0, 54).y, 1296.0);
        assert_eq!(v.hit_slop(), 12.0);
    }

    #[test]
    fn full_scale_respects_max_dimension() {
        assert_eq!(vp().full_scale(5000), 1.0);
        assert!((vp().full_scale(1272) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn keys_and_mouse_map_to_app_events() {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert!(matches!(key_action(key(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_action(key(KeyCode::Esc)), Action::Escape));
        assert!(matches!(
            key_action(key(KeyCode::Char('f'))),
            Action::App(AppEvent::FitView)
        ));
        assert!(matches!(
            key_action(key(KeyCode::Char('+'))),
            Action::App(AppEvent::Zoom(z)) if z > 0.0
        ));
        assert!(matches!(
            key_action(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        ));

        let m = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        assert!(matches!(
            mouse_action(m, &vp()),
            Action::App(AppEvent::Scroll(p, 1.0)) if p == Vec2::new(84.0, 60.0)
        ));
    }
}
