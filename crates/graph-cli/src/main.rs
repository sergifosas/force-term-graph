//! `graph` — force-directed graph viewer for the terminal.

mod config;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use graph_core::AppState;
use graph_render::{PngSpeed, Renderer};
use graph_sources::{FsSource, JsonSource, MarkdownSource, Source, load_all};
use graph_term::{PaneOptions, Protocol, run_pane};

/// Built-in sample graph, used by `graph demo`.
const DEMO_JSON: &str = include_str!("../../../tests/fixtures/small.json");

#[derive(Parser, Debug)]
#[command(
    name = "graph",
    version,
    about = "Obsidian-style force-directed graph viewer for WezTerm"
)]
struct Cli {
    #[command(subcommand)]
    source: Option<SourceCmd>,

    /// Render headlessly to a PNG file instead of displaying.
    #[arg(long, global = true, value_name = "FILE")]
    png: Option<PathBuf>,

    /// Image size for --png, as WIDTHxHEIGHT.
    #[arg(long, global = true, default_value = "1600x1000")]
    size: Size,

    /// Display inside the current terminal pane (default in WezTerm).
    #[arg(long, global = true, conflicts_with = "window")]
    pane: bool,

    /// Open a native GPU window: sharp at any size, animates at the
    /// display's refresh rate.
    #[arg(long, global = true)]
    window: bool,

    /// Terminal image protocol: kitty or iterm2 (auto-detected by default).
    #[arg(long, global = true)]
    protocol: Option<Protocol>,

    /// Extra config file, layered over the defaults and user config.
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Compute the final layout first instead of animating it in the pane.
    #[arg(long, global = true)]
    no_animate: bool,

    /// Frame-rate cap in the pane (default from config: 60).
    #[arg(long, global = true)]
    fps: Option<f32>,

    /// Show fps and frame timings in the status line, and a summary on exit.
    #[arg(long, global = true)]
    stats: bool,

    /// Benchmark: animate continuously for SECS seconds, then print stats.
    #[arg(long, global = true, value_name = "SECS", hide = true)]
    bench: Option<f32>,

    /// Resolution of in-motion frames, 0.1–1.0 (default from config: 1.0).
    /// Lower = more fps in the pane, softer while moving; a sharp frame is
    /// always drawn once motion stops.
    #[arg(long, global = true)]
    scale: Option<f32>,

    /// Maximum physics steps before rendering a static frame.
    #[arg(long, global = true, default_value_t = 5_000)]
    max_steps: usize,
}

/// With no subcommand, `graph` shows the Markdown notes under the current
/// directory.
#[derive(Subcommand, Debug)]
enum SourceCmd {
    /// Markdown vault: notes linked by [[wikilinks]], relative links and #tags.
    Md {
        /// Vault directory.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Don't create #tag nodes.
        #[arg(long)]
        no_tags: bool,
    },
    /// Directory tree: folders contain their files. Respects .gitignore.
    Fs {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Maximum depth below DIR.
        #[arg(long)]
        depth: Option<usize>,
        /// Include hidden and git-ignored entries.
        #[arg(long)]
        all: bool,
        /// Stop after this many nodes.
        #[arg(long, default_value_t = 2_000)]
        max_nodes: usize,
    },
    /// Load a JSON graph ({"nodes": [...], "edges": [...]}); `-` reads stdin.
    Json { path: PathBuf },
    /// Show the built-in sample graph.
    Demo,
}

fn load_graph(cmd: Option<&SourceCmd>) -> Result<graph_core::Graph> {
    let default_md = SourceCmd::Md {
        dir: ".".into(),
        no_tags: false,
    };
    let cmd = cmd.unwrap_or(&default_md);
    let source: Box<dyn Source> = match cmd {
        SourceCmd::Demo => {
            return Ok(JsonSource::parse(DEMO_JSON)?
                .namespaced("demo")
                .into_graph());
        }
        SourceCmd::Json { path } => Box::new(JsonSource::new(path)),
        SourceCmd::Md { dir, no_tags } => {
            let mut s = MarkdownSource::new(dir);
            s.tags = !no_tags;
            Box::new(s)
        }
        SourceCmd::Fs {
            dir,
            depth,
            all,
            max_nodes,
        } => {
            let mut s = FsSource::new(dir);
            s.max_depth = *depth;
            s.all = *all;
            s.max_nodes = *max_nodes;
            Box::new(s)
        }
    };
    let graph = load_all(&[source])?;
    if graph.is_empty() {
        match cmd {
            SourceCmd::Md { dir, .. } => bail!(
                "no Markdown notes found in {}.\n\
                 Try `graph md <vault>`, `graph fs {}` for the directory tree, or `graph demo`.",
                dir.display(),
                dir.display()
            ),
            _ => bail!("the graph is empty — nothing to show"),
        }
    }
    if let Some(limit) = graph
        .nodes()
        .first()
        .and_then(|n| n.meta.get("truncated_at"))
    {
        eprintln!("note: stopped at {limit} nodes; use --max-nodes or --depth to adjust");
    }
    Ok(graph)
}

#[derive(Debug, Clone, Copy)]
struct Size(u32, u32);

impl FromStr for Size {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (w, h) = s
            .split_once(['x', 'X'])
            .ok_or("expected WIDTHxHEIGHT, e.g. 1600x1000")?;
        let parse = |v: &str| {
            v.trim()
                .parse::<u32>()
                .ok()
                .filter(|n| (16..=16_384).contains(n))
        };
        match (parse(w), parse(h)) {
            (Some(w), Some(h)) => Ok(Size(w, h)),
            _ => Err("width and height must be integers between 16 and 16384".into()),
        }
    }
}

#[cfg(feature = "gui")]
fn run_window_frontend(app: AppState, cli: &Cli) -> Result<()> {
    if cli.no_animate {
        let mut app = app;
        app.settle(cli.max_steps);
        return run_window_frontend_inner(app, cli);
    }
    run_window_frontend_inner(app, cli)
}

#[cfg(feature = "gui")]
fn run_window_frontend_inner(app: AppState, cli: &Cli) -> Result<()> {
    let title = match &cli.source {
        Some(SourceCmd::Md { dir, .. }) | Some(SourceCmd::Fs { dir, .. }) => {
            format!("graph — {}", dir.display())
        }
        Some(SourceCmd::Json { path }) => format!("graph — {}", path.display()),
        Some(SourceCmd::Demo) => "graph — demo".into(),
        None => "graph — .".into(),
    };
    graph_gui::run_window(
        app,
        graph_gui::WindowOptions {
            title,
            stats: cli.stats,
            bench: cli.bench.map(std::time::Duration::from_secs_f32),
            ..Default::default()
        },
    )
}

#[cfg(not(feature = "gui"))]
fn run_window_frontend(_app: AppState, _cli: &Cli) -> Result<()> {
    bail!("this build has no window support (built without the `gui` feature)")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::load(cli.config.as_deref())?;

    let graph = load_graph(cli.source.as_ref())?;

    let viewport = (cli.size.0 as f32, cli.size.1 as f32);
    let mut app = AppState::new(graph, cfg.physics, cfg.style, viewport);
    let mut renderer = Renderer::new()?;

    if let Some(path) = &cli.png {
        app.settle(cli.max_steps);
        let png = renderer.render_png(&app.scene(), PngSpeed::Small)?;
        std::fs::write(path, png).with_context(|| format!("writing {}", path.display()))?;
        eprintln!(
            "wrote {} ({} nodes, {} edges, {}x{})",
            path.display(),
            app.graph.node_count(),
            app.graph.edge_count(),
            cli.size.0,
            cli.size.1
        );
        return Ok(());
    }

    // Frontend: flags beat config; "auto" = pane in an image-capable terminal,
    // otherwise the GPU window.
    let in_image_terminal = std::io::stdout().is_terminal() && Protocol::detect().is_some();
    let frontend = cfg.render.frontend.to_ascii_lowercase();
    let use_window = if cli.window {
        true
    } else if cli.pane || cli.protocol.is_some() {
        false
    } else {
        match frontend.as_str() {
            "window" => true,
            "pane" => false,
            "auto" => !in_image_terminal,
            other => bail!("invalid render.frontend `{other}` (expected auto, pane or window)"),
        }
    };
    if use_window {
        return run_window_frontend(app, &cli);
    }

    if !std::io::stdout().is_terminal() {
        bail!("stdout is not a terminal; use --window, or --png <file> for headless output");
    }
    // Explicit choice: flag beats config. `None` lets the pane ask the terminal.
    let protocol = match cli.protocol {
        Some(p) => Some(p),
        None if cfg.render.protocol.eq_ignore_ascii_case("auto") => None,
        None => Some(
            cfg.render
                .protocol
                .parse::<Protocol>()
                .map_err(anyhow::Error::msg)
                .context("invalid render.protocol in config")?,
        ),
    };
    if protocol.is_none() && Protocol::detect().is_none() && !cli.pane {
        bail!(
            "no supported terminal image protocol detected.\n\
             Use --window, run inside WezTerm, pass --protocol kitty|iterm2, or use --png <file>."
        );
    }
    if cli.no_animate {
        app.settle(cli.max_steps);
    }
    let report = run_pane(
        &mut app,
        &mut renderer,
        &PaneOptions {
            protocol,
            fps: cli.fps.unwrap_or(cfg.render.fps),
            interactive_scale: cli.scale.unwrap_or(cfg.render.interactive_scale),
            max_dimension: cfg.render.max_dimension,
            stats: cli.stats,
            bench: cli.bench.map(std::time::Duration::from_secs_f32),
        },
    )?;
    if cli.stats || cli.bench.is_some() {
        eprintln!("{}", report.summary());
    }
    Ok(())
}
