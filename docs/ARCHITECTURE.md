# RustUI Architecture

RustUI separates reactivity, layout, rendering, text, platform windowing, and widgets into six acyclic layers. Each layer depends only on the layers above it, enabling standalone headless testing of signals, layout, shaping, and offscreen rendering without opening a window.

```
rustui-core      (Node tree, SlotMap signals, glitch-free scheduler, View API)
    │
    ▼
rustui-layout    (Proposal/response protocol, stacks, Grid, LazyVStack, RTL)
    │
    ▼
rustui-render    (DisplayList, batched wgpu SDF + Vello, GlyphAtlas, DamageTracker)
    │
    ▼
rustui-text      (Parley shaping, Fontique discovery, Ropey editing, IME)
    │
    ▼
rustui-platform  (winit windows, HitTester, FocusManager, Gestures, AccessKit, Simulator)
    │
    ▼
rustui-widgets   (Core, Input, Collection, Container widgets, Theme, Styles)
    │
    ▼
rustui           (Public facade, prelude, App, CLI)
```

## Frame Pipeline

1. **Platform Events**: `winit` (or `HeadlessSimulator`) delivers pointer, scroll, keyboard, IME, resize, scale-factor, or theme events.
2. **Event & Task Handling**: `GestureRecognizer` / `FocusManager` invoke node callbacks (`on_tap`, `on_key`, `on_ime_commit`, etc.). Background `cx.spawn` tasks drain via `Runtime::poll_tasks()`.
3. **Glitch-Free Signal Flush**:
   - `State::set` / `update` marks downstream `Memo`s dirty in a min-heap ordered by dependency depth.
   - `Runtime::flush_reactive()` evaluates dirty memos at most once in topological order, then updates only dirty `If`/`Switch`/`ForEach` branches and reactive node property updaters.
4. **Incremental Layout**:
   - `mark_dirty` propagates `LAYOUT` upward until reaching a fixed-size boundary (`Frame::fixed`), marking ancestors with `DESCENDANT_LAYOUT`.
   - `LayoutEngine::layout()` skips clean subtrees in O(1) and negotiates `size`/`place` only for dirty branches and visible `LazyVStack` rows.
5. **Post-Layout Effects**: `Runtime::flush_effects()` runs pending `Effect` callbacks after layout completes, never during `View::body`.
6. **Display List & GPU Presentation**:
   - `emit_display_list()` walks visible nodes, resolves `Text` glyph runs via `ParleyTextPainter`, invokes `Canvas` callbacks, and optionally overlays `InspectorOverlay` bounds and `DamageTracker` regions.
   - `WgpuRenderer` coalesces instances into minimal pipeline/texture batches, uploads new glyphs to `GlyphAtlas`, renders `DrawPath` commands via `Vello`, and presents to the `wgpu::Surface` (or offscreen RGBA buffer).
7. **Accessibility Sync**: `AccessKitBridge` emits a `TreeUpdate` to AT-SPI / NSAccessibility / UI Automation whenever active.
