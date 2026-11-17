# RustUI — Project Specification

Nov 17, 2026 · @Ace

## 1. Vision, goals and non-goals

RustUI is a declarative, GPU-rendered desktop UI framework for Rust with SwiftUI-style ergonomics: views are cheap values, modifiers chain, and state drives the screen.

**Goals**

- SwiftUI-like authoring: `body()`, modifier chains, stacks, `State`/`Binding`.
- Fine-grained reactivity: a state change updates only the nodes that read it.
- Own renderer on wgpu, with a small display-list API between layout and GPU.
- One codebase on Linux (X11 and Wayland), macOS and Windows from the first milestone.
- First-class text: shaping, editing, IME, selection, accessibility.
- Fast enough for terminals, editors and large tables (60 fps, virtualized lists).

**Non-goals (v1)**

- Web, mobile and embedded targets.
- A visual designer or hot-reload GUI builder.
- Native platform widgets; RustUI draws everything itself.
- A bundled async runtime or networking stack.

**Design principles**

1. Declarative first: the user never mutates the node tree by hand.
2. Single UI thread; background work returns results through a channel into state.
3. No hidden global state; everything hangs off an `App` and its `Runtime`.
4. Layers are separable: layout, render and platform crates can be tested alone.
5. Stay stable at the edges: the public API is small and changes rarely, internals may churn.

## 2. Architecture and crate layout

RustUI is a stack of six layers. Each depends only on the ones above it in the table, so layout and render can be tested without a window.

| Layer | Crate | Responsibility | Key dependencies |
| --- | --- | --- | --- |
| Core | `rustui-core` | Node tree, signals, scheduler, identity | `slotmap` |
| Layout | `rustui-layout` | Proposal/response protocol, stacks, frame, alignment | core |
| Render | `rustui-render` | Scene graph, display list, wgpu backend, glyph atlas | `wgpu` |
| Text | `rustui-text` | Shaping, line breaking, editing model, IME state | `parley` |
| Platform | `rustui-platform` | Windows, input, clipboard, menus, accessibility bridge | `winit`, `accesskit` |
| Widgets | `rustui-widgets` | Button, TextField, List, ScrollView, Table | all of the above |

A facade crate `rustui` re-exports the public API and a `prelude`.

&#91;embedded content: crate architecture · 6 crates in 3 tiers\]

Arrows run from a crate to the crates it builds on; the table above lists each crate's key dependencies.

**Frame pipeline**

1. Platform delivers events (input, resize, redraw request).
2. Event handlers run and call `State::set`, which marks subscribed nodes dirty.
3. The scheduler re-runs only dirty node closures; structural nodes rebuild their subtree.
4. Layout runs on dirty subtrees and their ancestors as needed.
5. Paint emits a display list; only changed regions are re-emitted.
6. The wgpu backend uploads, draws and presents.

**Threading.** One UI thread owns the tree and the state arena. Background tasks send messages through a channel; a `cx.spawn` helper delivers results back as state updates.

## 3. Reactivity model

RustUI uses signals for state and a retained node tree for structure. `body()` runs once to build nodes; afterwards only closures that read a changed signal re-run.

**State primitives**

| Type | Purpose | Notes |
| --- | --- | --- |
| `State<T>` | Local mutable value | `Copy` handle into a slotmap arena |
| `Binding<T>` | Read/write view of a state | Derived with `state.binding()` or `.map()` |
| `Memo<T>` | Cached derived value | Re-computes only when inputs change |
| `Observable` | Model objects with many fields | Derive macro generates per-field signals (post-v1 sugar) |
| `Effect` | Side effect on change | Runs after layout, never during build |

**Rules**

1. `get()` inside a node closure or memo records a dependency on the current subscriber.
2. `set()` and `update()` mark subscribers dirty; notifications are batched per event.
3. Updates are glitch-free: a memo re-runs at most once per flush, in dependency order.
4. Reading a state outside a tracking scope is allowed and does not subscribe.
5. Dropping a node disposes its subscriptions and any states it owns.

**State ownership.** v1 uses hook-style `cx.state(init)`, slotted by call order within a view's `body`. Conditional calls are a compile-time lint in the macro layer. There is no field-based state derive in v1.

**Dynamic structure.** `If`, `Switch` and `ForEach` are the only primitives that rebuild subtrees. `ForEach` requires an explicit stable ID per item, so reorders move nodes instead of recreating them.

## 4. View API

Views are plain structs. A composite view implements `View::body`; a primitive view builds a retained node directly.

```rust
pub trait View {
    fn body(self, cx: &mut Cx) -> impl View;
}

pub trait Primitive {
    fn build(self, cx: &mut BuildCx) -> NodeId;
}

pub trait ViewExt: View + Sized {
    fn padding(self, v: f32) -> Padding<Self>;
    fn frame(self, f: Frame) -> Framed<Self>;
    fn background(self, c: Color) -> Background<Self>;
    fn on_tap(self, f: impl Fn() + 'static) -> OnTap<Self>;
}
```

**Example**

```rust
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
```

**Design decisions**

- **Children** are tuples (implemented by macro up to 12 elements) or a `vstack![...]` macro; `Vec<impl View>` is supported for homogeneous lists.
- **Modifiers** are wrapper types, so layout treats them as ordinary nodes. The type-erased `AnyView` is available at container boundaries to cut compile times.
- **Reactive text and values** take closures (`Fn() -> T`) so the framework can subscribe them. Static values are accepted through `Into`.
- **Environment**: `cx.env::<Theme>()` reads values inherited from ancestors, such as theme, locale and text direction; `.environment(value)` sets them.
- **Events**: gestures (`on_tap`, `on_hover`, `on_drag`, `on_key`) attach as modifiers and run on the UI thread.
- **Identity**: structural position gives default identity; `.id(key)` overrides it.

## 5. Layout system

Layout follows SwiftUI's negotiation: the parent proposes a size, the child picks its own size, and the parent places the child.

```rust
pub trait Layout {
    fn size(&self, proposed: Size<Option<f32>>, children: &mut Children) -> Size<f32>;
    fn place(&self, bounds: Rect, children: &mut Children);
}
```

`None` in a proposal means "your ideal size"; `Some(0.0)` means "as small as you can"; `Some(f32::INFINITY)` means "as large as you like".

**Built-in containers**

| View | Behavior |
| --- | --- |
| `VStack`, `HStack` | Distribute along one axis with spacing and cross-axis alignment; flexible children share leftover space by layout priority |
| `ZStack` | Overlay children with a 2D alignment |
| `Spacer` | Flexible empty space with a minimum length |
| `Grid` | Rows and columns with column sizing rules |
| `ScrollView` | Proposes unbounded size on its scroll axis |
| `LazyVStack` | Virtualized: only visible rows are built |

**Modifiers that affect layout:** `padding`, `frame(min/ideal/max)`, `fixed_size`, `aspect_ratio`, `offset`, `layout_priority`, `alignment_guide`.

**Requirements**

- Custom layouts implement `Layout` and can read `LayoutValue` metadata set by children.
- Results are cached per node and invalidated by dirty flags, so a text change re-measures only that node and its ancestors up to the first fixed-size boundary.
- Pixel snapping happens at paint time using the window scale factor.
- Right-to-left layouts mirror leading/trailing automatically.

## 6. Rendering

The renderer is owned by RustUI and sits behind a narrow display-list interface, so the backend can change without touching layout or widgets.

**Display list primitives**

- Filled and stroked rounded rectangles (SDF-based, anti-aliased).
- Paths: lines, curves, arbitrary fills, via the Vello GPU compute renderer.
- Images and nine-slice images.
- Glyph runs from the text crate, drawn from a glyph atlas.
- Shadows, blurs and gradients (linear, radial).
- Clip stacks (rect and rounded rect) and layers with opacity and blend mode.

**Backend (wgpu)**

| Platform | Primary API | Fallback |
| --- | --- | --- |
| Linux | Vulkan | OpenGL |
| macOS | Metal | none |
| Windows | DirectX 12 | Vulkan |

**Techniques**

1. Batch draws by pipeline and texture to keep draw calls low.
2. Cache glyphs in an atlas (subpixel positioning on, LCD AA optional).
3. Damage tracking: repaint only dirty regions where the platform allows partial presents; otherwise redraw the full frame.
4. Retain GPU resources per node subtree for static content such as images.
5. HiDPI: all geometry is in logical pixels; scale factor applied at submit time.

**Custom drawing.** A `Canvas` view exposes the display list to user code, which is what a terminal grid or chart would use. A `Shader` view lets advanced users supply a wgpu pipeline for a rectangle.

## 7. Text, input and focus

Text is the riskiest subsystem, so it gets its own crate and a milestone before widgets.

**Text rendering**

- Shaping and bidi through `parley`, with font fallback and system font discovery (fontconfig, CoreText, DirectWrite).
- Rich text: attributed runs for weight, style, color, size, underline and strikethrough.
- Line breaking, truncation (`line_limit`, `truncation_mode`), and per-line metrics exposed to layout.
- Emoji and color fonts supported in the glyph atlas.

**Editing model**

- A rope-backed buffer with cursor, selection ranges, undo/redo grouping and grapheme-aware movement.
- `TextField` (single line), `TextEditor` (multi line), and a lower-level `EditableText` trait for custom editors.
- Clipboard, drag selection, double and triple click, and platform key bindings.

**IME and composition**

- Pre-edit text rendered inline with underline, and candidate window positioned from the caret rect.
- Must work on macOS (NSTextInputClient), Windows (TSF/IMM), and Linux (IBus, Fcitx via Wayland text-input v3 and XIM).

**Input and focus**

| Area | Spec |
| --- | --- |
| Pointer | Hover, press, drag, scroll with inertia, pointer capture |
| Keyboard | Key events with physical and logical keys, shortcuts, `on_key` modifier |
| Focus | Focus tree, Tab / Shift+Tab traversal, `focusable()` and `focused(binding)` |
| Hit testing | Front-to-back on the node tree with clip and transform awareness |
| Gestures | Tap, long press, drag, pinch (trackpad), with simultaneous-gesture rules |

## 8. Platform layer and accessibility

RustUI targets Linux, macOS and Windows together. `winit` provides windows and events; platform-specific code fills the gaps it leaves behind a `Platform` trait.

Wayland is the priority Linux path; X11 support follows once the Wayland path is stable.

| Capability | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Windowing | X11 and Wayland (`winit`) | AppKit (`winit`) | Win32 (`winit`) |
| Menus | DBus global menu where available; in-window fallback | Native menu bar | Native menu bar |
| Clipboard and drag-drop | `arboard`, Wayland data-device | `NSPasteboard` | OLE |
| File dialogs | XDG portal | `NSOpenPanel` | `IFileDialog` |
| IME | Wayland text-input v3, IBus, Fcitx | `NSTextInputClient` | TSF |
| Accessibility | AT-SPI via AccessKit | NSAccessibility via AccessKit | UI Automation via AccessKit |
| Notifications and tray | DBus | `NSUserNotification` | Toast / shell notify |
| Window chrome | Client-side decorations on Wayland, server-side on X11 | Native title bar, optional transparent | Native, optional custom frame |

**Window API**

- Multiple windows, each with its own root view and focus state, sharing one runtime.
- Options: size limits, resizable, transparent, always-on-top, fullscreen, custom title bar.
- Events: resize, scale-factor change, occlusion, close request, theme change (light/dark).

**Accessibility.** Every node can expose a role, label, value and actions through AccessKit. Built-in widgets provide defaults. `.accessibility_label()`, `.accessibility_role()` and `.accessibility_hidden()` modifiers override them. Keyboard-only operation and screen-reader traversal are release criteria, not stretch goals.

**Testing across platforms.** CI builds and runs headless render tests on all three operating systems from the first milestone.

## 9. Widgets, styling and animation

**Widget set by milestone**

| Tier | Widgets |
| --- | --- |
| Core (M3) | `Text`, `Image`, `Button`, `Toggle`, `Divider`, `Spacer`, stacks |
| Input (M5) | `TextField`, `TextEditor`, `Slider`, `Picker`, `Checkbox`, `SecureField` |
| Collections (M6) | `ScrollView`, `List`, `LazyVStack`, `Table`, `Tree` |
| Containers (M7) | `NavigationStack`, `TabView`, `SplitView`, `Sheet`, `Popover`, `Menu`, `Toolbar` |

Every widget is built from public primitives, so users can replace or restyle any of them.

**Styling and theming**

- A `Theme` value in the environment holds colors, typography scale, spacing and corner radii. Light and dark variants follow the system setting by default.
- Semantic colors (`Color::primary()`, `Color::surface()`, `Color::accent()`) resolve from the theme.
- Style protocols (`ButtonStyle`, `ToggleStyle`, `ListStyle`) let a subtree change how a widget looks without changing its behavior.
- Fonts: system font by default, bundled fonts through a resource loader.

**Animation**

- `.animation(Animation::spring(), value: state)` animates property changes caused by that state.
- `with_animation(Animation::ease_in_out(0.25), || state.set(...))` animates everything that changes inside the closure.
- Animatable properties: position, size, opacity, color, corner radius, transforms.
- Transitions for insertion and removal: `.transition(.opacity)`, `.slide`, `.scale`, `.move(edge)`.
- The scheduler drives animations from the frame clock and requests redraws only while something is animating. Reduced-motion settings are honored.

**Resources.** Image loading (PNG, JPEG, SVG via `resvg`, WebP), asset bundling macro, and async image loading with placeholders.

## 10. Tooling, testing and performance targets

**Developer tooling**

- `rustui` CLI: `new` project template, `run`, and a `doctor` command that checks GPU drivers and platform dependencies.
- Inspector overlay (debug builds): node tree, layout bounds, dirty regions, signal dependency graph.
- Hot reload is a post-v1 stretch goal (likely through a dylib-based dev mode).
- Clear compile errors: custom diagnostics for common mistakes such as calling `cx.state` conditionally.

**Testing strategy**

| Level | Approach |
| --- | --- |
| Unit | Signals, scheduler and layout run without a window |
| Layout | Snapshot tests of node geometry for each container |
| Visual | Headless wgpu render to PNG, compared per platform with a tolerance |
| Interaction | Synthetic event driver: click, type, drag, focus; assert on state and tree |
| Accessibility | Assert the AccessKit tree for each built-in widget |
| Fuzz | Layout inputs and text editing operations |

**Performance targets (reference: mid-range laptop, 1080p)**

| Scenario | Target |
| --- | --- |
| Idle app | 0 redraws, under 1% CPU |
| Single state change in a 10,000-node tree | Under 1 ms to update, 8 ms frame total |
| Scrolling a virtualized list of 1,000,000 rows | 60 fps, no allocation per frame |
| Terminal grid, 200 x 60 cells, full-screen output | 60 fps |
| Cold start to first frame | Under 150 ms |
| Hello-world binary (release, stripped) | Under 15 MB |

**Release and quality**

- Semantic versioning with a documented stability policy; `0.x` until after M8.
- MSRV policy: latest stable minus two releases.
- Dual license MIT / Apache-2.0.
- Docs: API reference, a guide book, and an examples gallery (counter, todo, text editor, dashboard, terminal).

## 11. Roadmap and milestones

Each milestone ends with a runnable demo on Linux, macOS and Windows. A milestone is done only when its exit criteria pass on all three. Durations assume one full-time engineer and are rough.

| # | Milestone | Scope | Exit criteria | Est. |
| --- | --- | --- | --- | --- |
| M1 | Window and pixels | `winit` window, wgpu surface, rects, images, resize, HiDPI | Colored rounded rects and an image render identically on 3 OSes | 3 weeks |
| M2 | Text | Shaping, glyph atlas, rich text, fonts, bidi | Paragraph with mixed fonts and emoji wraps and scales correctly | 4 weeks |
| M3 | Reactive core and layout | Signals, node tree, scheduler, `VStack`/`HStack`/`ZStack`, modifiers, `Button`, `Text` | The Counter example runs; one signal change updates one node (verified by inspector) | 6 weeks |
| M4 | Input and focus | Pointer, keyboard, hit testing, focus traversal, scroll | Keyboard-only use of a form with buttons and toggles | 3 weeks |
| M5 | Text editing and IME | `TextField`, `TextEditor`, selection, undo, IME on all 3 OSes | Type Japanese, Chinese and emoji input correctly in all three | 6 weeks |
| M6 | Collections | `ScrollView`, `LazyVStack`, `List`, `Table`, `ForEach` diffing | 1,000,000-row list scrolls at 60 fps | 4 weeks |
| M7 | Containers, theming, animation | Navigation, tabs, split view, sheets, menus, `Theme`, springs, transitions | A multi-window demo app with light/dark themes | 6 weeks |
| M8 | Accessibility and polish | AccessKit tree, screen-reader pass, docs, CLI, examples, perf pass | Screen reader can operate the demo app; performance targets met | 5 weeks |

Total is roughly 37 weeks of focused work to a public `0.1` release. Dogfooding starts at M6, when a terminal view built on `Canvas` and `TextEditor` becomes a realistic test.

**Release plan**

1. `0.1` alpha after M8: API explicitly unstable, feedback wanted.
2. `0.2`: derive macros, hot reload, hardening from real apps.
3. `1.0`: after two apps ship on it and the API has been stable for a full release cycle.

## 12. Risks and open questions

**Risks**

| Risk | Impact | Mitigation |
| --- | --- | --- |
| IME and text input differ per OS | Blocks real apps; schedule slip in M5 | Dedicated milestone; study `winit`, Zed and Xilem implementations; test with real IMEs from day one |
| Compile times from generic view types | Poor developer experience | `AnyView` at container boundaries; avoid deep type nesting in widgets; measure in CI |
| Signal ownership and lifetimes in Rust | Leaks, panics, unclear errors | Arena-owned state, scoped disposal, clear debug messages; heavy testing of dispose paths |
| Scope too large for one person | Project stalls before usable | Ship vertical slices per milestone; dogfood early; cut widgets before core quality |
| Accessibility left late | Hard to retrofit | Node roles and labels are part of the node struct from M3 |
| Platform quirks (Wayland, fractional scaling, GPU drivers) | Rendering bugs on some machines | CI matrix, software fallback, `doctor` command |
| Competing frameworks (Xilem, Iced, Floem, Slint, GPUI) | Low adoption | Pick a clear niche: SwiftUI ergonomics with GPU-first performance and strong text |

**Open questions**

- [ ] Text stack: `cosmic-text` or `parley`? Use parley.
- [ ] State API: hook-style `cx.state()` only.
- [ ] `body(&self)` versus consuming `self` (as Xilem does); affects closures and moves. Use self as Xilem does.
- [ ] Path rendering: tessell ation with `lyon` first, or a GPU compute renderer such as Vello from the start? Vello, GPU compute renderer
- [ ] Which Linux windowing path gets priority if Wayland and X11 conflict? Wayland
- [ ] Project governance and license: MIT/Apache-2.0 dual, solo or open to contributors from the start? solo
