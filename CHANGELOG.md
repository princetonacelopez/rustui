# Changelog

All notable changes to RustUI are documented in this file.

## [0.1.0] - 2026-11-17

### Added
- `rustui-core`: Fine-grained reactive signals (`State`, `Binding`, `Memo`, `Effect`, `Observable`), retained slotmap node tree, glitch-free scheduler, `View` / `Primitive` / `ViewExt` APIs, `If` / `Switch` / `ForEach` dynamic control flow, hook-order diagnostics, and animations.
- `rustui-layout`: SwiftUI-style proposal/response `Layout` protocol, `VStack`, `HStack`, `ZStack`, `Spacer`, `Grid`, `ScrollView`, `LazyVStack`, layout modifiers, RTL mirroring, and pixel snapping.
- `rustui-text`: `parley`-backed text shaping, rich text spans, line breaking/truncation, bidi support, `ropey`-backed `TextBuffer` with undo/redo grouping and grapheme movement, and inline IME pre-edit & candidate positioning.
- `rustui-render`: Narrow display list API, batched WGSL SDF + Vello GPU renderer, subpixel & emoji glyph atlas, damage tracking, PNG/JPEG/WebP/SVG resource loader, debug inspector overlay, and headless offscreen rendering.
- `rustui-platform`: Cross-platform `winit` windowing (Linux Wayland/X11, macOS, Windows), pointer/keyboard/gesture hit-testing, Tab/Shift+Tab focus management, `arboard` clipboard, menus/dialogs/notifications bridges, `accesskit` bridge, and synthetic `HeadlessSimulator`.
- `rustui-widgets`: All 4 widget tiers (`Text`, `Image`, `Button`, `Toggle`, `Divider`, `Spacer`, `TextField`, `TextEditor`, `SecureField`, `Slider`, `Picker`, `Checkbox`, `ScrollView`, `LazyVStack`, `List`, `Table`, `Tree`, `NavigationStack`, `TabView`, `SplitView`, `Sheet`, `Popover`, `Menu`, `Toolbar`, `Canvas`, `Shader`), light/dark `Theme`, and spring/easing animations.
- `rustui`: Facade crate with `prelude`, `App`, `rustui` CLI (`new`, `run`, `doctor`), 5 examples (`counter`, `todo`, `text_editor`, `dashboard`, `terminal`), and full test/benchmark/fuzz suites.
