# force-term-graph

An Obsidian-style, force-directed graph viewer for the terminal, written in Rust.
It renders real anti-aliased graphics and shows them inside a WezTerm pane
through terminal image protocols. Markdown vaults, filesystem trees and
custom JSON can all be shown with the same engine.

```bash
graph                         # Markdown notes under the current folder
graph md ~/vault              # an Obsidian vault: [[wikilinks]], links, #tags
graph fs . --depth 3          # directory tree (respects .gitignore)
graph json data.json          # your own graph; `-` reads stdin
graph demo                    # built-in sample
graph md ~/vault --png out.png --size 1600x1000   # headless export
```

### Markdown

- Each `.md` file becomes a node. Its colour comes from its top-level folder.
- **Links:** `[[Note]]`, `[[Note|alias]]`, `[[Note#heading]]`,
  `[[folder/Note]]` and relative links like `[text](other.md)`. A bare name
  resolves like Obsidian does: case-insensitive, preferring a note in the
  same folder.
- **Unresolved links** appear as grey placeholder nodes.
- **Tags:** `#tags` become tag nodes; `--no-tags` turns them off.
- **Not counted as links:** anything in code blocks, inline code or
  frontmatter, and embedded attachments like `![[img.png]]`.
- **Skipped folders:** hidden ones such as `.obsidian`, and anything in
  `.gitignore`.

## Status

| Phase | State |
|---|---|
| 0: workspace, CI, terminal capability probe | ✅ |
| 1: graph model, physics with cooling, camera, scene | ✅ |
| MVP 0.1: JSON → layout → PNG → WezTerm pane | ✅ |
| 2: interactive pane (animation, mouse, drag-to-pin) | ✅ |
| 3: Markdown (`[[wikilinks]]`, tags) and filesystem sources | ✅ |
| 4: live reload (`notify`) | planned |
| 5: search, filters, open-in-editor via `wezterm cli` | planned |
| 6a: native GPU window (egui/wgpu), refresh-rate animation | ✅ |
| 6b: Barnes-Hut for large graphs | planned |

## Architecture

```
crates/
├── graph-core     model · physics · camera · scene · app state   (no I/O, no terminal, no GPU)
├── graph-sources  Markdown · filesystem · JSON → GraphData
├── graph-render   Scene → pixels (tiny-skia + fontdue, embedded Fira Sans)
├── graph-term     image protocols, pixel-size detection, pane loop + frame pipeline
├── graph-gui      native GPU window (eframe/egui on wgpu), same Scene, same controls
└── graph-cli      the `graph` binary: clap, layered config, wiring
```

Dependencies only point downward (`cli → term/sources → render → core`), so the
engine can't import anything terminal-specific even by accident.

- **The model and the layout are separate.** `Graph` uses dense `NodeId`
  indices, namespaced keys (`json:rust`, `md:notes/a.md`) and placeholder
  nodes for links that don't resolve. `Layout` holds positions, velocities
  and pins.
- **Physics.** The force model mixes Fruchterman-Reingold and d3-force:
  - repulsion falls off as `1/d`;
  - edges act as springs with a rest length;
  - a weak gravity pulls everything toward the centre;
  - friction damps the motion.

  It runs on a fixed timestep, and it *cools* (alpha decay). Once the layout
  is settled, `tick()` returns `false` and frontends stop redrawing.
- **Scene.** All styling lives in `graph-core`: colours, degree-scaled node
  sizes, culling, labels and hover highlighting. It produces a plain list of
  shapes, and every frontend is just a painter for that list.

## Pane or window

`graph` can draw in two places. Both use the same engine and the same
controls.

- **Pane** (default inside WezTerm): the graph is an image in your terminal
  pane, limited by how fast the terminal can decode images. That's ~41 fps
  at full sharpness in WezTerm.
- **Window** (`graph --window`): a native GPU window (wgpu). It's sharp at
  any size and synced to your display. Measured at 177 fps on a 180 Hz
  monitor. Physics runs at 60 steps/s, and the window blends between steps,
  so motion stays smooth at high refresh rates.

To make the window the default, set this in your config:

```toml
[render]
frontend = "window"
```

Or bind it in WezTerm:

```lua
{ key = 'g', mods = 'CTRL|ALT', action = wezterm.action.SpawnCommandInNewWindow {
    args = { 'graph', '--window' },
} },
```

## Using the pane

The layout animates into place and then stops. Once it has settled, `graph`
uses no CPU until you do something.

| Input | Action |
|---|---|
| hover | highlight a node and its neighbours (details in the status line) |
| drag a node | move it (it stays pinned while held, and its neighbours follow) |
| drag the background | pan |
| scroll · `+` `-` | zoom (scrolling zooms around the cursor) |
| arrows · `h` `j` `k` `l` | pan |
| `f` | fit the whole graph |
| `r` | re-run the layout |
| `Esc` | deselect; press again to quit |
| `q` | quit |

Frames render at the pane's native resolution, with a 60 fps cap. Rendering,
PNG encoding and terminal output each run on their own thread, so input
never waits on the terminal. If the terminal falls behind, stale frames are
dropped and the newest one is shown.

The limit is how fast the terminal can take in images. Measured in WezTerm
on Windows with a 2544×1296 pane:

| `--scale` | resolution | iTerm2 (default) | Kitty |
|---|---|---|---|
| 1.0 | 2544×1296 | ~41 fps | ~28 fps |
| 0.75 | 1908×972 | ~47 fps | ~45 fps |
| 0.6 | 1526×778 | ~54 fps | ~57 fps |

`--scale` (or `render.interactive_scale`) trades sharpness *while moving*
for frame rate. A sharp frame is always drawn once the motion stops.
`--stats` shows the live numbers in the status line.

## WezTerm

The protocol is chosen automatically:
- **WezTerm:** iTerm2 inline images, which measured fastest there.
- **kitty:** the Kitty protocol.
- **Unknown terminals:** `graph` asks whether they support Kitty. It reads
  the reply before input handling starts, because on Windows terminal
  replies arrive as keystrokes.

To force a protocol, use `--protocol kitty|iterm2` or `render.protocol`.

To get the pane's pixel size, `graph` uses `wezterm cli list` together with
`$WEZTERM_PANE`, because Windows ConPTY doesn't report pixel sizes.

You can open the graph in a split pane with a keybinding:

```lua
config.keys = {
  { key = 'g', mods = 'CTRL|SHIFT', action = wezterm.action.SplitHorizontal {
      args = { 'graph' },
  } },
}
```

### Capability probe

Run this **inside WezTerm** to check what your setup supports: image
protocols, pixel size and mouse events. It writes `probe-report.txt`.

```bash
cargo run -p graph-term --example probe
```

## Configuration

Built-in defaults are in [config/default.toml](config/default.toml). Settings
are layered in this order, each overriding the previous one key by key:

1. the built-in defaults;
2. your user config: `%APPDATA%\graph\config.toml` on Windows,
   `~/.config/graph/config.toml` elsewhere;
3. a file passed with `--config file.toml`.

Unknown keys are rejected, so typos don't silently do nothing.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p graph-core --example tune   # physics tuning harness
cargo install --path crates/graph-cli    # puts `graph` on your PATH
```

The label font is Fira Sans, licensed under the SIL Open Font License; see
[crates/graph-render/assets/OFL.txt](crates/graph-render/assets/OFL.txt).
