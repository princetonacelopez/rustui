# RustUI Developer Guide

## Views and Modifiers

Composite views implement `View::body(self, cx: &mut Cx) -> impl View`, consuming `self` by value:

```rust
use rustui::prelude::*;

struct ProfileCard {
    name: String,
}

impl View for ProfileCard {
    fn body(self, cx: &mut Cx) -> impl View {
        let likes = cx.state(0u32);
        HStack::new(12, (
            Text::new(self.name).font(Font::headline()),
            Spacer::new(),
            Button::new(move || format!("Likes: {}", likes.get()), move || {
                likes.update(|n| *n += 1);
            }),
        ))
        .padding(12)
        .background(Color::surface())
        .corner_radius(8)
    }
}
```

## Reactive State Primitives

- **`State<T>`**: `Copy` handle allocated with `cx.state(init)`. Call `state.get()` inside reactive closures to subscribe, `state.get_untracked()` outside tracking, and `state.set(...)` / `state.update(...)` to mutate.
  - *Rule*: Always call `cx.state(...)` unconditionally in the same order across rebuilds (use `rustui_check_hooks!` for compile-time guardrails).
- **`Binding<T>`**: Two-way read/write reference created via `state.binding()`, `Binding::new(get, set)`, or projected via `.map(read, write)`.
- **`Memo<T>`**: Cached derived computation (`cx.memo(|| ...)`) that re-computes at most once per flush in topological dependency order.
- **`EffectHandle`**: Side effect (`cx.effect(|| ...)`) scheduled after layout completes.
- **`Observable`**: Use `impl_observable!` to define model structs with per-field `State<F>` signals.

## Dynamic Control Flow

- `If::new(|| cond.get(), || ThenView).else_view(|| ElseView)`
- `Switch::new(|| tab.get()).case(0, || TabA).case(1, || TabB).default(|| Fallback)`
- `ForEach::new(|| items.get(), |item| item.id, |item| RowView { item })`: Stable IDs move and preserve existing child nodes and their internal `State` across reorders.

## Environment & Theming

Attach values to any subtree with `.environment(value)` and read them inside child `View::body` with `cx.env::<T>()`:

```rust
App::new(RootView).theme(Theme::dark()).run()
```

Use semantic colors (`Color::primary()`, `Color::surface()`, `Color::accent()`, `Color::border()`) or swap `ButtonStyle`, `ToggleStyle`, and `ListStyle` in the environment.

## Background Work (`cx.spawn`)

Keep heavy I/O or computation off the UI thread using `cx.spawn`:

```rust
cx.spawn(
    move || expensive_query(),
    move |result| results_state.set(result),
);
```

## Animations & Transitions

- `.animation(Animation::spring(), my_state)`
- `with_animation(Animation::ease_in_out(0.25), || opacity.set(0.0))`
- `.transition(Transition::Move(Edge::Trailing))`

## Custom Layout and Custom Drawing

Implement `rustui::layout::Layout` and wrap children in `CustomLayoutView::new(MyLayout, (...))`, reading child metadata via `child.layout_value::<MyTag>()`.

Use `Canvas::new(|rect, display_list| ...)` for immediate display-list drawing (charts, terminal grids, custom graphics) or `Shader::new(wgsl_source)` for custom WGSL surfaces.

## Accessibility

Every node exposes role, label, value, and actions to screen readers via AccessKit:

- Built-in widgets provide sensible defaults (`Button`, `Toggle`, `Checkbox`, `Slider`, `Picker`, `TextField`, `TextEditor`, `SecureField`, `List`, `Table`, `Tree`, `TabView`).
- Override defaults with `.accessibility_label(...)`, `.accessibility_role(...)`, `.accessibility_value(...)`, `.accessibility_hidden(true)`, and `.focusable()` / `.focused(binding)`.
