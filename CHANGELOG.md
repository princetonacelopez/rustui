# Changelog

All notable changes to RustUI are documented in this file.

## [0.2.0] - 2026-11-18

### Added
- **Proc-Macro Ergonomics (`rustui-macros`)**:
  - `#[derive(Observable)]` + `ObservableModel` trait (`cx.model(...)`) generating per-field reactive getters, untracked getters, setters, updaters, bindings, and state handles.
  - `#[preview("Label")]` attribute macro generating preview descriptors for `rustui dev` and IDEs like Anvil.
- **Reactive, Async & Lifecycle Primitives (`rustui-core`)**:
  - `create_effect_cleanup` / `Cx::effect_cleanup` teardown hooks.
  - `TimerId` & scheduler timer ticks (`Cx::timer`).
  - `AsyncExecutor`, `Resource<T>`, `ResourceState<T>` (`Cx::resource`, `Cx::spawn_future`).
  - Memo dependency cycle detection panic and bounded `64`-pass rebuild loop guard.
  - High-contrast (`is_high_contrast`), forced-colors (`is_forced_colors`), and dynamic `text_scale_factor` environment switches.
  - `O(1)` child attachment (`attached_to_parent`) and `O(1)` subtree disposal (`owned_reactives`) tested cleanly at `100,000+` nodes.
  - `ViewExt` additions: `.accessibility_hint`, `.accessibility_live`, `.accessibility_sort_priority`, `.accessibility_custom_action`, `.accessibility_element_combine`, `.on_size_change`, `.on_geometry`, `.grid_cell_columns`, `.grid_cell_rows`, `.draggable`, `.on_drop`, and `#[track_caller]` source locations.
- **Layout, Render & Telemetry (`rustui-layout`, `rustui-render`)**:
  - Multi-column/row spanning `GridLayout`, `ViewThatFits`, layout cache hit/miss metrics, and dynamic `text_scale_factor` font measurement scaling.
  - `GlyphAtlas` LRU eviction under memory budget (`evict_if_over_budget`, `page_count`).
  - `InspectorServer` non-blocking local TCP JSON snapshot server (`InspectorFrameSnapshot`, `InspectorNodeSnapshot` with `source_file`/`source_line`/`source_col`) and Chrome/Perfetto trace export (`export_perfetto_trace`).
- **Text & IDE Editing (`rustui-text`, `rustui-widgets`)**:
  - O(log N) rope operations across editing, cursor navigation, bracket matching (`find_matching_bracket`), and selection slicing (eliminating O(N) whole-rope copies).
  - Multi-cursor selection engine (`add_cursor_above`, `add_cursor_below`, `add_cursor_next_match`, `set_column_selection`, atomic multi-cursor undo/redo), persistent `AnchorId` / `AnchorBias`, and UTF-8 / UTF-16 LSP line-column conversion helpers.
  - `EditorSurface` + `EditorBufferState` widget with syntax token spans, diagnostic squiggles, inline inlay hints, code folding toggles, git gutter diff markers, breakpoints, matching bracket highlights, minimap, keyboard/IME, and AccessKit support.
- **Desktop Widgets, Theming & Platform Services (`rustui-platform`, `rustui-widgets`)**:
  - `SplitView` vertical & collapsible modes, `TabBar`, `StatusBar`, `CommandPalette`, `Alert`, `ConfirmationDialog`, `FileDialog`, `DisclosureGroup`, `OutlineGroup`, `ProgressView`, `Gauge`, `ViewThatFits`, and `TerminalView`.
  - `Theme::high_contrast()`, `Theme::to_toml_string()`, `Theme::from_toml_str()`.
  - `KeybindingRegistry` multi-stroke chord matcher with TOML load/save, OS + intra-app drag-and-drop (`HeadlessSimulator::drop_files`, `drag_and_drop`), `HotReloadHost`, and `AccessKitBridge` support for `Meter`, `AlertDialog`, `Status`, descriptions, live regions, and custom actions.
- **CLI & Showcase (`rustui`)**:
  - Upgraded `rustui` CLI with `new --template app|multi-window|lib`, `dev`, `run`, `build`, `doctor`, `bench`, and `bundle --format app|appimage|flatpak|msix`.
  - Added `gallery` example showcasing all 0.2 widgets, themes, and 200% scaling.

### Changed
- `ScrollView` now shrink-wraps content along its non-scrolling cross axis instead of absorbing transient stack `fair_share` height/width proposals.
- Single-child `ContainerKind::Fragment` (`Switch`/`If`) preserves the full proposed height when placing its active branch.
- Bumped workspace crate versions from `0.1.0` to `0.2.0`.

## [0.1.0] - 2026-11-17

### Added
- Initial release featuring `rustui-core`, `rustui-layout`, `rustui-render`, `rustui-text`, `rustui-platform`, `rustui-widgets`, and `rustui` facade crate + `counter`, `todo`, `text_editor`, `dashboard`, and `terminal` examples.
