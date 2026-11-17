# How to Build and Run RustUI

This guide covers system prerequisites, building from source, using the `rustui` CLI, running the examples gallery, taking headless visual snapshots, and running the verification suite.

## Prerequisites

- **Rust toolchain**: Rust `1.98` or newer (`rustup` automatically selects the stable toolchain from `rust-toolchain.toml`).
- **Linux**:
  - Runtime shared libraries: Vulkan ICD (Mesa RADV/ANV/Lavapipe or NVIDIA), Wayland client / X11 (`libwayland-client.so.0`, `libX11.so.6`, `libxkbcommon.so.0`), and Fontconfig (`libfontconfig.so.1`).
  - Note: RustUI enables `wayland-dlopen` and `fontconfig-dlopen` so building from source does **not** require `-dev` C header packages on standard Linux systems.
- **macOS**: Xcode Command Line Tools (`Metal` and `CoreText` are included with macOS).
- **Windows**: MSVC build tools (`DirectX 12` / `Vulkan` and `DirectWrite` are included with Windows).

## Step 1: Clone and Verify Your Environment

```bash
git clone https://github.com/princetonacelopez/rustui.git
cd rustui
cargo run -p rustui --bin rustui -- doctor
```

Example `rustui doctor` output:

```text
RustUI Doctor Report
====================
OS:                  Linux (x86_64)
Display server:      Wayland (wayland-0) + X11 (:0)
Linux priority:      Wayland (priority), X11 fallback
GPU adapter:         AMD Radeon Graphics (RADV REMBRANDT) (backend: Vulkan, type: IntegratedGpu)
Vello compute:       available
Discovered fonts:    205 families
IME backend:         Wayland text-input v3 / IBus / Fcitx / XIM
Accessibility:       AT-SPI via AccessKit
Clipboard:           OS clipboard ready
```

## Step 2: Run the Examples Gallery

RustUI includes five runnable examples covering every subsystem:

| Example | Command (Windowed) | Highlights |
| --- | --- | --- |
| `counter` | `cargo run -p rustui --example counter` | `State`, `VStack`/`HStack`, fine-grained text updates, `Button` |
| `todo` | `cargo run -p rustui --example todo` | `TextField`, `Checkbox`, keyed `ForEach` diffing, `Picker`, `Memo` |
| `text_editor` | `cargo run -p rustui --example text_editor` | `Toolbar`, `SplitView`, `Tree`, `TextEditor`, rope undo/redo, CJK/Emoji IME |
| `dashboard` | `cargo run -p rustui --example dashboard` | `NavigationStack`, `TabView`, `Table`, 1M-row `LazyVStack`, `Canvas` chart, `Sheet`, `Popover`, `Menu` |
| `terminal` | `cargo run -p rustui --example terminal` | 200x60 terminal grid rendered via `Canvas`, `TextField`, dark `Theme` |

### Running via the `rustui` CLI

```bash
cargo run -p rustui --bin rustui -- run --example dashboard
```

### Headless Execution, Inspector Overlay, and PNG Snapshots

Every example and `App::new(...).run()` supports headless verification flags and environment variables:

```bash
# Run any example headlessly without opening a GUI window
cargo run -p rustui --example counter -- --headless

# Or via environment variable
RUSTUI_HEADLESS=1 cargo run -p rustui --example todo

# Render with the debug inspector overlay (node bounds + dirty regions + stats badge) and save PNG
cargo run -p rustui --example dashboard -- --headless --inspector --snapshot /tmp/dashboard.png

# Force deterministic CPU software rasterization fallback instead of GPU
RUSTUI_SOFTWARE_RENDER=1 cargo run -p rustui --example counter -- --headless --snapshot /tmp/sw.png
```

## Step 3: Create a New Application

```bash
cargo run -p rustui --bin rustui -- new my_desktop_app
cd my_desktop_app
cargo run
```

## Step 4: Run Tests, Benchmarks, Fuzzing, and Lints

Run the full test suite (unit tests across all 6 crates + performance targets + fuzz tests + AccessKit accessibility audit):

```bash
cargo test --workspace
```

Run specific verification suites:

```bash
# Performance targets (0 idle redraws, <1ms 10k-node update, 60fps 1M virtual list, 200x60 terminal)
cargo test -p rustui --test perf_targets

# Property/fuzz tests for layout negotiation and rope text editing
cargo test -p rustui --test fuzz_tests

# AccessKit roles, labels, actions, and keyboard-only traversal audit
cargo test -p rustui --test accessibility_audit
```

Run formatting and strict Clippy lints:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Generate local API documentation:

```bash
cargo doc --workspace --no-deps --open
```

Build a stripped release binary of the CLI:

```bash
cargo build --release -p rustui --bin rustui
```
