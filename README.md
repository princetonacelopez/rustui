# RustUI 0.2

[![CI](https://github.com/princetonacelopez/rustui/actions/workflows/ci.yml/badge.svg)](https://github.com/princetonacelopez/rustui/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

**RustUI** is a declarative, GPU-rendered desktop UI framework for Rust with SwiftUI-style ergonomics: views are cheap values, modifiers chain cleanly, and fine-grained signals drive the screen.

## Features (0.2)

- **SwiftUI-style authoring & proc macros**: `View::body(self, cx: &mut Cx) -> impl View`, `#[derive(Observable)]` (per-field `Copy` signal handles with `cx.model(...)`), `#[preview("Label")]`, modifier chains (`.padding`, `.frame`, `.background`, `.on_tap`, `.on_size_change`, `.on_geometry`, `.environment`), `ViewThatFits`, multi-column `Grid`, `State`/`Binding`/`Memo`/`Effect` with `create_effect_cleanup`, `TimerId`, and async `Resource<T>`.
- **Fine-grained, glitch-free reactivity**: `SlotMap`-backed signal arena (`O(1)` attach and subtree disposal verified at `100,000+` nodes), memo cycle detection, bounded rebuild loops, and topological post-layout `Effect` scheduling.
- **Proposal/response layout engine**: Two-pass size negotiation (`None` ideal, `Some(0.0)` min, `Some(INFINITY)` max) with `layout_priority`, fixed-size invalidation boundaries, layout cache metrics, 200% dynamic `text_scale_factor`, pixel snapping, RTL mirroring, and `1,000,000+` row virtualized `LazyVStack`.
- **Owned GPU renderer (`wgpu` + `Vello`)**: Narrow `DisplayList` API, batched instanced WGSL SDF rounded rects/shadows/gradients, `Vello` GPU compute vector paths, LRU-evicting glyph atlas (`GlyphAtlas`), local `InspectorServer` JSON endpoint with `#[track_caller]` source locations, Perfetto trace export (`export_perfetto_trace`), and an automatic software rasterizer fallback for headless CI.
- **IDE-grade text & IME (`rustui-text` + `EditorSurface`)**: `parley` + `fontique` shaping, rich text spans, bidi & complex scripts (CJK/Arabic/Emoji), O(log N) `ropey`-backed `TextBuffer` with multi-cursor editing, persistent `AnchorId` markers, UTF-8/UTF-16 LSP coordinate conversion, bracket matching, syntax spans, diagnostic squiggles, inlay hints, code folding, git diff gutter markers, breakpoints, and minimap.
- **Desktop IDE & app widgets**: `SplitView` (horizontal/vertical & collapsible), `TabBar`, `StatusBar`, `CommandPalette`, `Alert`, `ConfirmationDialog`, `FileDialog`, `DisclosureGroup`, `OutlineGroup`, `ProgressView`, `Gauge`, `TerminalView`, `Table`, `Tree`, `List`, `Toolbar`, `Menu`, `Popover`, `Sheet`, and TOML-configurable `Light`, `Dark`, and `High Contrast` themes.
- **Cross-platform & accessible**: Single codebase targeting Linux (Wayland priority + X11), macOS, and Windows via `winit`, OS + intra-app drag-and-drop, global `KeybindingRegistry`, `HotReloadHost`, `arboard` clipboard, touch/pinch gestures, and `AccessKit` (`AlertDialog`, `Meter`, `Status`, hints, live regions, custom actions).

## Architecture

RustUI 0.2 is organized into eight workspace crates:

| Layer | Crate | Responsibility | Key Dependencies |
| --- | --- | --- | --- |
| Macros | [`rustui-macros`](crates/rustui-macros) | `#[derive(Observable)]`, `#[preview]` zero-dep proc macros | `proc_macro` |
| Core | [`rustui-core`](crates/rustui-core) | Retained node tree, signals, async/timers, scheduler, `View` | `slotmap` |
| Layout | [`rustui-layout`](crates/rustui-layout) | Proposal/response protocol, stacks, `Grid`, `ViewThatFits`, `LazyVStack` | `rustui-core` |
| Render | [`rustui-render`](crates/rustui-render) | Display list, `wgpu`/Vello backend, atlas LRU, `InspectorServer`, Perfetto | `wgpu`, `vello`, `swash`, `resvg`, `image` |
| Text | [`rustui-text`](crates/rustui-text) | Shaping, rich text, O(log N) rope buffer, multi-cursor, UTF-16, IME | `parley`, `fontique`, `ropey`, `unicode-bidi` |
| Platform | [`rustui-platform`](crates/rustui-platform) | Windows, keybindings, DnD, AccessKit, hot-reload host, simulator | `winit`, `accesskit`, `accesskit_winit`, `arboard` |
| Widgets | [`rustui-widgets`](crates/rustui-widgets) | Core, form, collection, container, `EditorSurface`, `TerminalView`, TOML `Theme` | all of the above |
| Facade & CLI | [`rustui`](crates/rustui) | Public `prelude`, `App`, `rustui` CLI, showcase examples | workspace crates |

## Quickstart

```rust
use rustui::prelude::*;

#[derive(Observable, Clone, Default)]
struct CounterModel {
    count: i32,
}

struct Counter;

impl View for Counter {
    fn body(self, cx: &mut Cx) -> impl View {
        let model = cx.model(CounterModel { count: 0 });
        VStack::new(12, (
            Text::new(move || format!("Count: {}", model.count())).font(Font::title()),
            HStack::new(8, (
                Button::new("-", move || model.update_count(|c| *c -= 1)),
                Button::new("+", move || model.update_count(|c| *c += 1)),
            )),
        ))
        .padding(16)
    }
}

fn main() -> Result<(), String> {
    App::new(Counter)
        .title("RustUI Counter")
        .size(400.0, 260.0)
        .run()
}
```

## How to Run

See [docs/RUNNING.md](docs/RUNNING.md) and [CHANGELOG.md](CHANGELOG.md) for details.

### 1. Check Environment Diagnostics

```bash
cargo run -p rustui --bin rustui -- doctor
```

### 2. Run the Examples & 0.2 Widget Gallery

Run any example in a desktop window:

```bash
cargo run -p rustui --example gallery
cargo run -p rustui --example counter
cargo run -p rustui --example todo
cargo run -p rustui --example text_editor
cargo run -p rustui --example dashboard
cargo run -p rustui --example terminal
```

Or run headlessly with offscreen PNG snapshot & inspector:

```bash
cargo run -p rustui --example gallery -- --headless --inspector --snapshot /tmp/gallery.png
```

### 3. Developer CLI (`new`, `dev`, `run`, `build`, `doctor`, `bench`, `bundle`)

```bash
cargo run -p rustui --bin rustui -- new my_app --template app
cargo run -p rustui --bin rustui -- dev --example gallery
cargo run -p rustui --bin rustui -- build --release
cargo run -p rustui --bin rustui -- bench
cargo run -p rustui --bin rustui -- bundle --format appimage --no-build
```

### 4. Run Tests, Lints, Licenses, and API Docs

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
RUSTUI_SOFTWARE_RENDER=1 cargo test -p rustui-render
cargo test --workspace
cargo deny check licenses
```

## Documentation

- [Changelog](CHANGELOG.md)
- [0.2 Migration Guide](docs/MIGRATION-0.2.md)
- [Platform & Widget Compatibility Matrix](docs/COMPATIBILITY.md)
- [Running & CLI Guide](docs/RUNNING.md)
- [Developer Guide Book](docs/GUIDE.md)
- [Architecture Overview](docs/ARCHITECTURE.md)
- [Stability & MSRV Policy](docs/STABILITY_POLICY.md)
- [Project Specification (0.2)](RustUI%200.2%20%E2%80%94%20Specification.md)

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
