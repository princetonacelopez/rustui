# RustUI

[![CI](https://github.com/princetonacelopez/rustui/actions/workflows/ci.yml/badge.svg)](https://github.com/princetonacelopez/rustui/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

**RustUI** is a declarative, GPU-rendered desktop UI framework for Rust with SwiftUI-style ergonomics: views are cheap values, modifiers chain, and fine-grained signals drive the screen.

## Features

- **SwiftUI-style authoring**: `View::body(self, cx: &mut Cx) -> impl View`, modifier chains (`.padding`, `.frame`, `.background`, `.on_tap`, `.environment`), `VStack`/`HStack`/`ZStack`/`Grid`, `State`/`Binding`/`Memo`/`Effect`.
- **Fine-grained, glitch-free reactivity**: `SlotMap`-backed signal arena (`State<T>` is `Copy`). A state mutation updates only subscribed nodes in topological dependency order; post-layout `Effect`s never run during view construction.
- **Proposal/response layout engine**: Two-pass size negotiation (`None` ideal, `Some(0.0)` min, `Some(INFINITY)` max) with `layout_priority`, fixed-size invalidation boundaries, pixel snapping, RTL mirroring, and `1,000,000+` row virtualized `LazyVStack`.
- **Owned GPU renderer (`wgpu` + `Vello`)**: Narrow `DisplayList` API between layout and GPU, batched instanced WGSL SDF rounded rects/shadows/gradients, `Vello` GPU compute vector paths, subpixel & color-emoji `GlyphAtlas`, damage tracking, PNG/JPEG/WebP/SVG (`resvg`) resource loading, and an automatic software rasterizer fallback for headless CI.
- **First-class text & IME**: `parley` + `fontique` shaping, rich text spans, bidi & complex scripts (CJK/Arabic/Emoji), `ropey`-backed `TextBuffer` with grapheme-aware navigation and coalesced undo/redo, and inline IME pre-edit composition + candidate positioning.
- **Cross-platform & accessible**: Single codebase targeting Linux (Wayland priority + X11), macOS, and Windows via `winit`, `arboard` clipboard, and `AccessKit` accessibility (`Role`, labels, values, actions, keyboard-only `Tab`/`Shift+Tab` traversal).

## Architecture

RustUI is organized into six layered crates plus the `rustui` facade crate and CLI:

| Layer | Crate | Responsibility | Key Dependencies |
| --- | --- | --- | --- |
| Core | [`rustui-core`](crates/rustui-core) | Retained node tree, signals, scheduler, `View` API | `slotmap` |
| Layout | [`rustui-layout`](crates/rustui-layout) | Proposal/response protocol, stacks, `Grid`, `LazyVStack` | `rustui-core` |
| Render | [`rustui-render`](crates/rustui-render) | Display list, `wgpu`/Vello backend, glyph atlas, damage, inspector | `wgpu`, `vello`, `swash`, `resvg`, `image` |
| Text | [`rustui-text`](crates/rustui-text) | Shaping, rich text, line breaking, rope editing, IME state | `parley`, `fontique`, `ropey`, `unicode-bidi` |
| Platform | [`rustui-platform`](crates/rustui-platform) | Windows, input/gestures, focus, clipboard, AccessKit, simulator | `winit`, `accesskit`, `accesskit_winit`, `arboard` |
| Widgets | [`rustui-widgets`](crates/rustui-widgets) | Core, input, collection, and container widgets, `Theme`, styles | all of the above |
| Facade & CLI | [`rustui`](crates/rustui) | Public `prelude`, `App`, `rustui` CLI (`new`, `run`, `doctor`), examples | workspace crates, `clap` |

## Quickstart

```rust
use rustui::prelude::*;

struct Counter;

impl View for Counter {
    fn body(self, cx: &mut Cx) -> impl View {
        let count = cx.state(0);
        VStack::new(12, (
            Text::new(move || format!("Count: {}", count.get())).font(Font::title()),
            HStack::new(8, (
                Button::new("-", move || count.update(|c| *c -= 1)),
                Button::new("+", move || count.update(|c| *c += 1)),
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

See [docs/RUNNING.md](docs/RUNNING.md) for full platform setup and troubleshooting instructions.

### 1. Check Environment Diagnostics

```bash
cargo run -p rustui --bin rustui -- doctor
```

### 2. Run the Examples Gallery

Run any example in a desktop window:

```bash
cargo run -p rustui --example counter
cargo run -p rustui --example todo
cargo run -p rustui --example text_editor
cargo run -p rustui --example dashboard
cargo run -p rustui --example terminal
```

Or run headlessly (renders offscreen without opening a window, with optional PNG snapshot and debug inspector):

```bash
cargo run -p rustui --example counter -- --headless --inspector --snapshot counter.png
```

### 3. Scaffold a New Project with the CLI

```bash
cargo run -p rustui --bin rustui -- new my_app
cd my_app && cargo run
```

### 4. Run Tests, Lints, and API Docs

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps --open
```

## Documentation

- [Running & CLI Guide](docs/RUNNING.md)
- [Developer Guide Book](docs/GUIDE.md)
- [Architecture Overview](docs/ARCHITECTURE.md)
- [Stability & MSRV Policy](docs/STABILITY_POLICY.md)
- [Project Specification](RustUI%20%E2%80%94%20Project%20Specification.md)

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
