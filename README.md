# force-term-graph

An Obsidian-style, force-directed graph viewer written in Rust. Type `graph`
in WezTerm to see how your Markdown notes, a directory tree or your own JSON
data connect, drawn as real anti-aliased graphics rather than ASCII.

It draws in one of two places, with the same engine and controls:

- **Pane:** inside your terminal pane, as an image (iTerm2 or Kitty
  protocol).
- **Window:** a native GPU window, sharp at any size and animated at your
  display's refresh rate.

```bash
graph                         # Markdown notes under the current folder
graph md ~/vault              # an Obsidian vault
graph fs . --depth 3          # a directory tree
graph json data.json          # your own graph (`-` reads stdin)
graph demo                    # built-in sample
graph md ~/vault --window     # native GPU window instead of the pane
graph md ~/vault --png out.png --size 1600x1000   # headless export
```

## Install

You need a Rust toolchain. On Windows you also need the MSVC C++ Build
Tools.

```bash
cargo install --path crates/graph-cli
```

This puts `graph` in `~/.cargo/bin`. For a lighter terminal-only build
without the GPU window:

```bash
cargo install --path crates/graph-cli --no-default-features
```

## Data sources

### Markdown: `graph md [DIR]` (also plain `graph`)

- **Nodes:** each `.md` file becomes a node, coloured by its top-level
  folder.
- **Links:** `[[Note]]`, `[[Note|alias]]`, `[[Note#heading]]`,
  `[[folder/Note]]` and relative links like `[text](other.md)`. A bare name
  resolves like Obsidian does: case-insensitive, preferring a note in the
  same folder.
- **Unresolved links** appear as grey placeholder nodes.
- **Tags:** `#tags` become tag nodes; `--no-tags` turns them off.
- **Ignored:** links in code blocks, inline code or frontmatter, and embedded
  attachments like `![[img.png]]`.
- **Skipped folders:** hidden ones such as `.obsidian`, and anything matched
  by `.gitignore`.

### Filesystem: `graph fs [DIR]`

Each folder links to its files and subfolders. Files are coloured by
extension. `.gitignore` and hidden entries are skipped unless you pass
`--all`. Use `--depth N` to limit depth, and `--max-nodes N` (default 2000)
to cap the size.

### JSON: `graph json FILE` (`-` for stdin)

```json
{
  "nodes": [
    { "id": "rust", "label": "Rust", "category": "language" },
    { "id": "wezterm" }
  ],
  "edges": [
    { "source": "rust", "target": "wezterm", "weight": 1.0 }
  ]
}
```

Only `id`, `source` and `target` are required. Each node also accepts an
optional `meta` object for arbitrary data. Edges that point at an unknown id
create a placeholder node.

## Controls

Both frontends use the same controls.

| Input | Action |
|---|---|
| hover | highlight a node and its neighbours (details in the status bar) |
| drag a node | move it (pinned while held; neighbours follow) |
| drag the background | pan |
| scroll · `+` `-` | zoom (scrolling zooms around the cursor) |
| arrow keys | pan (the pane also accepts `h` `j` `k` `l`) |
| `f` | fit the whole graph |
| `r` | re-run the layout |
| `Esc` | deselect; press again to quit |
| `q` | quit |

The layout animates into place and then stops. Once nothing is moving,
`graph` uses no CPU until you interact.

## Pane or window

| | Pane (default in WezTerm) | Window (`--window`) |
|---|---|---|
| Where | inside the terminal pane | separate native window |
| Drawing | CPU (tiny-skia) → PNG → terminal image protocol | GPU (wgpu) |
| Frame rate | limited by the terminal (~41 fps sharp in WezTerm) | display refresh rate (177 fps measured on a 180 Hz monitor) |

**Choosing which one opens.** With `render.frontend = "auto"` (the default),
`graph` opens in the pane when the terminal supports images. Otherwise it
opens the window. `--pane` and `--window` override this, and you can make
either one permanent:

```toml
# %APPDATA%\graph\config.toml (Windows) or ~/.config/graph/config.toml
[render]
frontend = "window"
```

### Pane performance

A full-resolution frame takes `graph` about 5 ms. Rendering, PNG encoding and
terminal output each run on their own thread, so input never waits on the
terminal. If the terminal falls behind, older frames are dropped and the
newest one is shown. The real limit is how fast the terminal decodes images.
Measured in WezTerm on Windows with a 2544×1296 pane:

| `--scale` | resolution | iTerm2 (default) | Kitty |
|---|---|---|---|
| 1.0 | 2544×1296 | ~41 fps | ~28 fps |
| 0.75 | 1908×972 | ~47 fps | ~45 fps |
| 0.6 | 1526×778 | ~54 fps | ~57 fps |

`--scale` (or `render.interactive_scale`) trades sharpness *while moving* for
frame rate. A sharp frame is always drawn once the motion stops. `--stats`
shows live fps and timings in the status bar, and prints a summary when you
quit.

### Window smoothness

Physics runs at a fixed 60 steps per second. The window blends positions
between steps, so motion stays smooth at 120/144/180 Hz.

## WezTerm integration

**Image protocol.** `graph` picks one automatically:

- **WezTerm:** iTerm2 inline images, which measured fastest there.
- **kitty:** the Kitty protocol.
- **Other terminals:** `graph` asks whether they support Kitty and reads the
  reply before input handling starts. On Windows, terminal replies arrive as
  keystrokes, so this matters.

To force one, use `--protocol kitty|iterm2` or `render.protocol`.

**Pixel size.** Windows ConPTY doesn't report the pane size in pixels, so
`graph` asks `wezterm cli list` for the size of `$WEZTERM_PANE`.

**Keybindings.** Add these to `wezterm.lua` to open the graph in a split
pane, or in a GPU window:

```lua
config.keys = {
  { key = 'g', mods = 'CTRL|SHIFT', action = wezterm.action.SplitHorizontal {
      args = { 'graph' },
  } },
  { key = 'g', mods = 'CTRL|ALT', action = wezterm.action.SpawnCommandInNewWindow {
      args = { 'graph', '--window' },
  } },
}
```

**Capability probe.** Run this *inside* the terminal you want to support. It
checks image protocols, pixel-size detection and mouse events, and writes
`probe-report.txt`:

```bash
cargo run -p graph-term --example probe
```

## Command-line options

| Option | Meaning |
|---|---|
| `--window` / `--pane` | choose the frontend |
| `--png FILE`, `--size WxH` | render the settled layout to a PNG and exit |
| `--protocol kitty\|iterm2` | force the terminal image protocol |
| `--scale 0.1–1.0` | pane resolution while moving |
| `--fps N` | pane frame-rate cap (default 60) |
| `--stats` | live fps and timings, and a summary on exit |
| `--no-animate` | compute the final layout before showing it |
| `--config FILE` | extra config layer |

Run `graph --help` or `graph <source> --help` to see all options.

## Configuration

Every setting, with comments, is in
[config/default.toml](config/default.toml):

- `[physics]`: forces, friction and cooling.
- `[style]`: colours, palette, node sizes and label rules.
- `[render]`: frontend, protocol, fps and scale.

Settings are layered, each layer overriding the previous one key by key:

1. the built-in defaults;
2. your user config: `%APPDATA%\graph\config.toml` on Windows,
   `~/.config/graph/config.toml` elsewhere;
3. a file passed with `--config file.toml`;
4. command-line flags.

Unknown keys are rejected, so a typo produces an error instead of being
silently ignored.

## Architecture

```
crates/
├── graph-core     model · physics · camera · scene · app state   (no I/O, no terminal, no GPU)
├── graph-sources  Markdown · filesystem · JSON → GraphData
├── graph-render   Scene → pixels (tiny-skia + fontdue, embedded Fira Sans)
├── graph-term     image protocols, pixel-size detection, pane loop, frame pipeline
├── graph-gui      native GPU window (eframe/egui on wgpu)
└── graph-cli      the `graph` binary: clap, layered config, frontend selection
```

Dependencies only point toward `graph-core`, so the engine can't import
anything terminal- or GPU-specific, even by accident.

- **Model and layout are separate.**
  - `Graph` uses dense `NodeId` indices and namespaced keys (`md:notes/a.md`,
    `fs:src`, `json:rust`). Unresolved links become placeholder nodes.
  - `Layout` holds positions, velocities and pins.
- **Physics** mixes Fruchterman-Reingold and d3-force:
  - repulsion falls off as `1/d`;
  - edges act as springs with a rest length;
  - a weak gravity pulls toward the centre;
  - friction damps the motion.

  It runs on a fixed timestep and *cools*, so the layout settles and
  frontends stop redrawing. The defaults were tuned by counting edge
  crossings with `examples/tune.rs`.
- **Scene.** All styling lives in `graph-core`: colours, node sizes scaled
  by number of links, culling, labels and hover highlighting. Its output is
  a plain list of shapes. Each frontend just paints that list, using
  tiny-skia for the pane and PNG, and egui for the window.
- **App state.** Each frontend turns its native input into `AppEvent`s, so
  hit-testing, dragging and zooming behave the same everywhere.

## Status

| Phase | State |
|---|---|
| 0: workspace, CI, terminal capability probe | ✅ |
| 1: graph model, physics with cooling, camera, scene | ✅ |
| 2: interactive pane (animation, mouse, drag-to-pin) | ✅ |
| 3: Markdown and filesystem sources | ✅ |
| 6a: native GPU window, refresh-rate animation | ✅ |
| 4: live reload when files change (`notify`) | planned |
| 5: search, filters, open a note in your editor via `wezterm cli` | planned |
| 6b: Barnes-Hut layout for large graphs (thousands of nodes) | planned |

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Measuring tools:

```bash
cargo run --release -p graph-core   --example tune         # layout quality per physics setting
cargo run --release -p graph-render --example frame_cost   # render + PNG encode cost per frame
graph fs . --stats                                         # live fps in the pane
graph fs . --window --stats                                # live fps in the window
```

CI runs formatting, clippy and the tests on Windows and Linux
([.github/workflows/ci.yml](.github/workflows/ci.yml)).

## License

The label font is Fira Sans, licensed under the SIL Open Font License. See
[crates/graph-render/assets/OFL.txt](crates/graph-render/assets/OFL.txt).
