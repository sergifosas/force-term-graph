//! Background frame pipeline, so the input/physics loop never waits on the
//! terminal:
//!
//! ```text
//! event loop ──FrameJob──▶ encoder thread ──EncodedFrame──▶ writer thread ──▶ stdout
//!              (mailbox)   render + PNG +    (mailbox)       escape codes
//!                          protocol encode
//! ```
//!
//! Each hand-off is a single-slot mailbox: submitting replaces an item that
//! hasn't been picked up yet. When the terminal can't keep up, stale frames
//! are dropped and the newest one is shown, so input latency stays low. The
//! two stages overlap, so throughput is limited by the slower stage instead
//! of their sum.
//!
//! All stdout writes during the session happen on the writer thread.

use std::io::{Write, stdout};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{cursor, queue, style, terminal};
use graph_core::Scene;
use graph_render::{PngSpeed, Renderer};

use crate::protocol::Protocol;

pub struct FrameJob {
    pub scene: Scene,
    pub scale: f32,
    pub cols: u16,
    pub image_rows: u16,
    pub status_row: u16,
    pub status: String,
    /// Clear the screen first (after a resize).
    pub clear: bool,
}

struct EncodedFrame {
    payload: Vec<u8>,
    /// Bytes of image data produced (PNG size), for stats.
    image_bytes: usize,
    cols: u16,
    status_row: u16,
    status: String,
    clear: bool,
    size: (u32, u32),
    render: Duration,
    encode: Duration,
}

/// Totals over the whole session.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameReport {
    pub frames: u64,
    /// Frames replaced before a pipeline stage got to them.
    pub dropped: u64,
    /// From the first frame shown to the last (startup excluded).
    pub elapsed: Duration,
    first_frame: Option<Instant>,
    last_frame: Option<Instant>,
    pub render: Duration,
    pub encode: Duration,
    pub write: Duration,
    pub bytes: u64,
    /// Size of the last frame, in pixels.
    pub frame_size: (u32, u32),
}

impl FrameReport {
    /// Frames per second between the first and last frame shown.
    pub fn fps(&self) -> f64 {
        self.frames.saturating_sub(1) as f64 / self.elapsed.as_secs_f64().max(1e-9)
    }

    fn avg_ms(&self, d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0 / self.frames.max(1) as f64
    }

    pub fn summary(&self) -> String {
        format!(
            "{}x{} px · {} frames in {:.1}s = {:.1} fps ({} dropped) · render {:.1} ms · \
             encode {:.1} ms · terminal write {:.1} ms · {:.0} KB/frame",
            self.frame_size.0,
            self.frame_size.1,
            self.frames,
            self.elapsed.as_secs_f64(),
            self.fps(),
            self.dropped,
            self.avg_ms(self.render),
            self.avg_ms(self.encode),
            self.avg_ms(self.write),
            self.bytes as f64 / 1024.0 / self.frames.max(1) as f64,
        )
    }
}

/// Single-slot, latest-wins hand-off between threads.
struct Mailbox<T> {
    slot: Mutex<(Option<T>, bool)>, // (item, quit)
    ready: Condvar,
}

impl<T> Mailbox<T> {
    fn new() -> Self {
        Self {
            slot: Mutex::new((None, false)),
            ready: Condvar::new(),
        }
    }

    /// Returns the unconsumed item this replaced, if any (it was dropped).
    fn put(&self, item: T) -> Option<T> {
        let mut s = self.slot.lock().unwrap();
        let replaced = s.0.replace(item);
        self.ready.notify_one();
        replaced
    }

    fn take(&self) -> Option<T> {
        let mut s = self.slot.lock().unwrap();
        loop {
            if s.1 {
                return None;
            }
            if let Some(item) = s.0.take() {
                return Some(item);
            }
            s = self.ready.wait(s).unwrap();
        }
    }

    fn close(&self) {
        self.slot.lock().unwrap().1 = true;
        self.ready.notify_all();
    }
}

pub struct Pipeline {
    jobs: Mailbox<FrameJob>,
    encoded: Mailbox<EncodedFrame>,
    report: Mutex<FrameReport>,
    error: Mutex<Option<String>>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            jobs: Mailbox::new(),
            encoded: Mailbox::new(),
            report: Mutex::default(),
            error: Mutex::default(),
        }
    }
}

impl Pipeline {
    /// Queues a frame, replacing one that hasn't started yet.
    pub fn submit(&self, job: FrameJob) {
        if self.jobs.put(job).is_some() {
            self.report.lock().unwrap().dropped += 1;
        }
    }

    pub fn shutdown(&self) {
        self.jobs.close();
        self.encoded.close();
    }

    /// Why the pipeline stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn report(&self) -> FrameReport {
        let mut r = *self.report.lock().unwrap();
        if let (Some(a), Some(b)) = (r.first_frame, r.last_frame) {
            r.elapsed = b - a;
        }
        r
    }

    fn fail(&self, e: anyhow::Error) {
        self.error.lock().unwrap().get_or_insert(format!("{e:#}"));
        self.shutdown();
    }

    /// Encoder stage: render + PNG + protocol escape sequence.
    pub fn run_encoder(&self, renderer: &mut Renderer, protocol: Protocol) {
        while let Some(job) = self.jobs.take() {
            match encode(&job, renderer, protocol) {
                Ok(frame) => {
                    if self.encoded.put(frame).is_some() {
                        self.report.lock().unwrap().dropped += 1;
                    }
                }
                Err(e) => return self.fail(e),
            }
        }
    }

    /// Writer stage: pushes encoded frames to the terminal. The write blocks
    /// while the terminal is busy, which is the pipeline's backpressure.
    pub fn run_writer(&self, stats: bool) {
        let mut live = LiveStats::default();
        while let Some(frame) = self.encoded.take() {
            let t = Instant::now();
            if let Err(e) = write_frame(&frame, stats.then(|| live.text())) {
                return self.fail(e);
            }
            let write = t.elapsed();
            live.record(&frame, write);
            let mut r = self.report.lock().unwrap();
            r.frames += 1;
            r.render += frame.render;
            r.encode += frame.encode;
            r.write += write;
            r.bytes += frame.image_bytes as u64;
            r.frame_size = frame.size;
            let now = Instant::now();
            r.first_frame.get_or_insert(now);
            r.last_frame = Some(now);
        }
    }
}

fn encode(job: &FrameJob, renderer: &mut Renderer, protocol: Protocol) -> Result<EncodedFrame> {
    let t0 = Instant::now();
    let pm = renderer.render_scaled(&job.scene, job.scale)?;
    let t1 = Instant::now();
    let png = graph_render::encode_png(&pm, PngSpeed::Fast)?;
    let payload = protocol.encode(&png, job.cols, job.image_rows);
    Ok(EncodedFrame {
        payload,
        image_bytes: png.len(),
        cols: job.cols,
        status_row: job.status_row,
        status: job.status.clone(),
        clear: job.clear,
        size: (pm.width(), pm.height()),
        render: t1 - t0,
        encode: t1.elapsed(),
    })
}

fn write_frame(f: &EncodedFrame, stats: Option<String>) -> Result<()> {
    let mut out = stdout().lock();
    if f.clear {
        queue!(out, terminal::Clear(terminal::ClearType::All))?;
    }
    queue!(out, cursor::MoveTo(0, 0))?;
    out.write_all(&f.payload)?;
    let status = match stats {
        Some(s) => format!(" {s} ·{}", f.status),
        None => f.status.clone(),
    };
    let status: String = status.chars().take(f.cols as usize).collect();
    queue!(
        out,
        cursor::MoveTo(0, f.status_row),
        terminal::Clear(terminal::ClearType::CurrentLine),
        style::SetForegroundColor(style::Color::DarkGrey),
        style::Print(status),
        style::ResetColor
    )?;
    out.flush()?;
    Ok(())
}

/// Rolling numbers for the `--stats` status line.
#[derive(Default)]
struct LiveStats {
    window_start: Option<Instant>,
    window_frames: u32,
    fps: f64,
    render_ms: f64,
    encode_ms: f64,
    write_ms: f64,
    kb: f64,
}

impl LiveStats {
    fn record(&mut self, f: &EncodedFrame, write: Duration) {
        let ema = |avg: &mut f64, v: f64| *avg = if *avg == 0.0 { v } else { *avg * 0.9 + v * 0.1 };
        ema(&mut self.render_ms, f.render.as_secs_f64() * 1000.0);
        ema(&mut self.encode_ms, f.encode.as_secs_f64() * 1000.0);
        ema(&mut self.write_ms, write.as_secs_f64() * 1000.0);
        ema(&mut self.kb, f.image_bytes as f64 / 1024.0);
        let start = *self.window_start.get_or_insert_with(Instant::now);
        self.window_frames += 1;
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            self.fps = self.window_frames as f64 / elapsed;
            self.window_start = Some(Instant::now());
            self.window_frames = 0;
        }
    }

    fn text(&self) -> String {
        format!(
            "{:.0} fps · draw {:.1} ms · png {:.1} ms · out {:.1} ms · {:.0} KB",
            self.fps, self.render_ms, self.encode_ms, self.write_ms, self.kb
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_keeps_only_the_latest_item() {
        let m = Mailbox::new();
        assert!(m.put(1).is_none());
        assert_eq!(m.put(2), Some(1), "unconsumed item is replaced");
        assert_eq!(m.take(), Some(2));
        m.close();
        assert_eq!(m.take(), None::<i32>);
    }
}
