# RustUI 0.2 Platform & Widget Compatibility Matrix

## Platform Backends

| Feature | Linux (Wayland priority / X11 fallback) | macOS (AppKit / Metal) | Windows (Win32 / DX12) | Headless CI (`HeadlessSimulator`) |
| --- | --- | --- | --- | --- |
| GPU Renderer (`wgpu` + `Vello`) | Vulkan + Software fallback | Metal + Software fallback | DX12 + Software fallback | `RUSTUI_SOFTWARE_RENDER=1` RGBA/PNG |
| Windowing & High-DPI | `winit` | `winit` | `winit` | Configurable `scale_factor` & `text_scale_factor` |
| IME Pre-edit & Commit | Wayland text-input / IBus / Fcitx | Cocoa NSTextInputClient | TSF / IMM32 | `ImeState` + synthetic events |
| Accessibility | AccessKit AT-SPI2 | AccessKit NSAccessibility | AccessKit UI Automation | Full `accesskit::TreeUpdate` verification |
| Clipboard & File Dialogs | `arboard` + system picker | `arboard` + `NSOpenPanel` | `arboard` + IFileDialog | Mocked & programmable |
| Drag & Drop (Intra + OS) | `DroppedFile` + `DragPayload` | `DroppedFile` + `DragPayload` | `DroppedFile` + `DragPayload` | `drop_files` / `drag_and_drop` |
| High Contrast & Reduced Motion | OS detection + runtime override | OS detection + runtime override | OS detection + runtime override | Programmable toggles |

## Widget Inventory (0.2.0)

- **Core**: `Text`, `Image`, `Button`, `Toggle`, `Divider`, `Spacer`, `VStack`, `HStack`, `ZStack`, `Grid`, `ViewThatFits`, `CustomLayoutView`, `Canvas`, `Shader`, `ProgressView`, `Gauge`, `TerminalView`
- **Inputs**: `TextField`, `SecureField`, `TextEditor`, `EditorSurface`, `Slider`, `Checkbox`, `Picker`
- **Collections**: `ScrollView`, `LazyVStack`, `List`, `Table`, `Tree`, `OutlineGroup`, `ForEach`
- **Containers & Chrome**: `NavigationStack`, `TabView`, `TabBar`, `SplitView`, `Toolbar`, `StatusBar`, `Menu`, `Popover`, `Sheet`, `CommandPalette`, `Alert`, `ConfirmationDialog`, `FileDialog`, `DisclosureGroup`
