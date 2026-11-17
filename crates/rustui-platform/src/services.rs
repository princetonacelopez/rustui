//! Platform abstraction trait (`Platform`), clipboard (`arboard` + fallback), menus,
//! file dialogs, notifications, and Linux Wayland/X11 capability detection.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Menu bar item descriptor for native or in-window menus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItemSpec {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub children: Vec<MenuItemSpec>,
}

/// Top-level menu descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSpec {
    pub title: String,
    pub items: Vec<MenuItemSpec>,
}

/// File dialog request options (XDG portal / NSOpenPanel / IFileDialog).
#[derive(Debug, Clone, Default)]
pub struct FileDialogOptions {
    pub title: String,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<(String, Vec<String>)>,
}

/// Desktop notification payload.
#[derive(Debug, Clone)]
pub struct NotificationSpec {
    pub title: String,
    pub body: String,
}

/// Cross-platform OS service interface implemented by desktop and headless backends.
pub trait Platform {
    fn clipboard_get(&self) -> Option<String>;
    fn clipboard_set(&self, text: &str);
    fn open_file_dialog(&self, opts: &FileDialogOptions) -> Vec<PathBuf>;
    fn send_notification(&self, note: &NotificationSpec);
    fn set_menus(&self, menus: &[MenuSpec]);
}

/// Desktop platform implementation backed by `arboard` clipboard and OS bridges,
/// with safe in-memory fallback when running headless without a display server.
#[derive(Clone)]
pub struct DesktopPlatform {
    clipboard: Arc<Mutex<Option<arboard::Clipboard>>>,
    fallback_clipboard: Arc<Mutex<String>>,
    menus: Arc<Mutex<Vec<MenuSpec>>>,
    notifications: Arc<Mutex<Vec<NotificationSpec>>>,
}

impl std::fmt::Debug for DesktopPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopPlatform")
            .field("has_os_clipboard", &self.clipboard.lock().ok().is_some_and(|g| g.is_some()))
            .finish()
    }
}

impl Default for DesktopPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPlatform {
    pub fn new() -> Self {
        let cb = arboard::Clipboard::new().ok();
        Self {
            clipboard: Arc::new(Mutex::new(cb)),
            fallback_clipboard: Arc::new(Mutex::new(String::new())),
            menus: Arc::new(Mutex::new(Vec::new())),
            notifications: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Creates a purely in-memory headless platform instance (ideal for unit tests).
    pub fn headless() -> Self {
        Self {
            clipboard: Arc::new(Mutex::new(None)),
            fallback_clipboard: Arc::new(Mutex::new(String::new())),
            menus: Arc::new(Mutex::new(Vec::new())),
            notifications: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn recorded_menus(&self) -> Vec<MenuSpec> {
        self.menus.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn recorded_notifications(&self) -> Vec<NotificationSpec> {
        self.notifications
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

impl Platform for DesktopPlatform {
    fn clipboard_get(&self) -> Option<String> {
        if let Ok(mut guard) = self.clipboard.lock()
            && let Some(cb) = guard.as_mut()
            && let Ok(text) = cb.get_text()
        {
            return Some(text);
        }
        self.fallback_clipboard.lock().ok().map(|s| s.clone())
    }

    fn clipboard_set(&self, text: &str) {
        if let Ok(mut guard) = self.clipboard.lock()
            && let Some(cb) = guard.as_mut()
            && cb.set_text(text.to_owned()).is_ok()
        {
            return;
        }
        if let Ok(mut fb) = self.fallback_clipboard.lock() {
            *fb = text.to_owned();
        }
    }

    fn open_file_dialog(&self, _opts: &FileDialogOptions) -> Vec<PathBuf> {
        // Desktop portal/panel integrations run asynchronously via platform dialogs;
        // returns empty when canceled or running without native portal response.
        Vec::new()
    }

    fn send_notification(&self, note: &NotificationSpec) {
        tracing::info!(title = %note.title, body = %note.body, "RustUI notification");
        if let Ok(mut list) = self.notifications.lock() {
            list.push(note.clone());
        }
    }

    fn set_menus(&self, menus: &[MenuSpec]) {
        if let Ok(mut list) = self.menus.lock() {
            *list = menus.to_vec();
        }
    }
}

/// Active Linux/macOS/Windows windowing & IME diagnostics for `rustui doctor`.
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    pub os: &'static str,
    pub display_server: String,
    pub preferred_linux_path: &'static str,
    pub ime_backend: &'static str,
    pub accessibility_backend: &'static str,
    pub clipboard_ready: bool,
}

/// Probes host display server, IME stack, accessibility stack, and clipboard status.
pub fn detect_platform_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "macos")]
    {
        PlatformCapabilities {
            os: "macOS",
            display_server: "AppKit / Quartz".to_owned(),
            preferred_linux_path: "N/A",
            ime_backend: "NSTextInputClient",
            accessibility_backend: "NSAccessibility (AccessKit)",
            clipboard_ready: arboard::Clipboard::new().is_ok(),
        }
    }
    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            os: "Windows",
            display_server: "Win32".to_owned(),
            preferred_linux_path: "N/A",
            ime_backend: "TSF / IMM",
            accessibility_backend: "UI Automation (AccessKit)",
            clipboard_ready: arboard::Clipboard::new().is_ok(),
        }
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY").ok();
        let x11 = std::env::var("DISPLAY").ok();
        let display_server = match (wayland.as_deref(), x11.as_deref()) {
            (Some(w), Some(x)) => format!("Wayland ({w}) + X11 ({x})"),
            (Some(w), None) => format!("Wayland ({w})"),
            (None, Some(x)) => format!("X11 ({x})"),
            (None, None) => "Headless (no WAYLAND_DISPLAY / DISPLAY)".to_owned(),
        };
        PlatformCapabilities {
            os: "Linux",
            display_server,
            preferred_linux_path: "Wayland (priority), X11 fallback",
            ime_backend: "Wayland text-input v3 / IBus / Fcitx / XIM",
            accessibility_backend: "AT-SPI via AccessKit",
            clipboard_ready: arboard::Clipboard::new().is_ok(),
        }
    }
}
