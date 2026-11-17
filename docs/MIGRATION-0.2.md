# Migrating from RustUI 0.1 to 0.2

RustUI 0.2 is source-compatible with 0.1 views while introducing major ergonomics, IDE widgets, and accessibility improvements.

## 1. Crate Version Bump

Update your dependency version in `Cargo.toml`:

```toml
[dependencies]
rustui = "0.2.0"
```

## 2. Adopt `#[derive(Observable)]` (Optional)

Instead of manually allocating many individual `cx.state(...)` hooks, group view/feature state into an observable struct:

```rust
use rustui::prelude::*;

#[derive(Observable, Clone, Default)]
struct ProfileState {
    name: String,
    notifications: bool,
}

impl View for ProfileView {
    fn body(self, cx: &mut Cx) -> impl View {
        let state = cx.model(ProfileState::default());
        Toggle::new("Notifications", state.notifications_binding())
    }
}
```

## 3. Rich Desktop & IDE Containers

- **`SplitView`**: now supports `SplitView::vertical(...)`, `.collapsible(true)`, and `.collapsed(binding)` in addition to `SplitView::horizontal(...)`.
- **`TabBar` / `StatusBar` / `CommandPalette`**: use `TabBar::new(tabs, active)` for reorderable/pinnable editor tabs, `StatusBar::new(...)` anchored at the bottom of your workspace `VStack`, and `CommandPalette::new(open, items, content)` for fuzzy command search.
- **`EditorSurface`**: pass `EditorBufferState` via `EditorSurface::new(binding)` when building multi-cursor code editors with syntax spans, inlay hints, squiggles, folding, git diff markers, and minimaps.

## 4. Accessibility Additions

New modifiers on any `View`:
- `.accessibility_hint("Explains action")`
- `.accessibility_live(AccessibilityLive::Polite)`
- `.accessibility_sort_priority(10.0)`
- `.accessibility_custom_action("Format Document", || ...)`
- `.accessibility_element_combine(true)`

New semantic controls expose `Role::Meter` (`Gauge`) and `Role::AlertDialog` (`Alert` / `ConfirmationDialog` / `CommandPalette`).

## 5. Text Scaling & TOML Themes

- Call `runtime.set_text_scale_factor(2.0)` to scale UI text cleanly up to 200%.
- Load and save theme presets using `Theme::from_toml_str(&src)` / `theme.to_toml_string()`, or activate `Theme::high_contrast()`.
