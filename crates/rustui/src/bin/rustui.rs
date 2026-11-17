//! RustUI 0.2 developer CLI (`rustui new|dev|run|build|doctor|bench|bundle`).

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, SystemTime};

fn print_usage() {
    println!(
        "\
rustui 0.2.0 — RustUI framework CLI

USAGE:
    rustui <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    new <name> [--template app|multi-window|lib]   Scaffold a new RustUI 0.2 project
    dev [--example <name>] [--once]                Run with RUSTUI_HOT_RELOAD=1 file watcher
    run [--example <name>] [--release] [-- ...]    Run the current RustUI project or example
    build [--release] [--target <triple>]          Compile the current project
    doctor                                         Inspect Rust toolchain, GPU, display & IDE tools
    bench                                          Run RustUI performance target benchmarks
    bundle [--format app|appimage|flatpak|msix]  Generate desktop bundle staging artifacts
"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateKind {
    App,
    MultiWindow,
    Lib,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BundleFormat {
    App,
    AppImage,
    Flatpak,
    Msix,
}

impl BundleFormat {
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "app" | "macos" => Ok(Self::App),
            "appimage" => Ok(Self::AppImage),
            "flatpak" => Ok(Self::Flatpak),
            "msix" | "windows" => Ok(Self::Msix),
            _ => Err(format!(
                "unknown bundle format '{s}' (expected app, appimage, flatpak, or msix)"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::AppImage => "appimage",
            Self::Flatpak => "flatpak",
            Self::Msix => "msix",
        }
    }
}

fn cmd_new(args: &[String]) -> Result<(), String> {
    let mut name: Option<String> = None;
    let mut template = TemplateKind::App;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                let t = args
                    .get(i)
                    .ok_or_else(|| "--template requires app, multi-window, or lib".to_string())?;
                template = match t.as_str() {
                    "app" => TemplateKind::App,
                    "multi-window" => TemplateKind::MultiWindow,
                    "lib" => TemplateKind::Lib,
                    other => {
                        return Err(format!(
                            "unsupported template '{other}' (expected app, multi-window, or lib)"
                        ));
                    }
                };
            }
            "--help" | "-h" => {
                println!("Usage: rustui new <name> [--template app|multi-window|lib]");
                return Ok(());
            }
            arg if !arg.starts_with('-') && name.is_none() => {
                name = Some(arg.to_string());
            }
            other => return Err(format!("unrecognized argument '{other}'")),
        }
        i += 1;
    }

    let project_name = name.ok_or_else(|| "missing project name: rustui new <name>".to_string())?;
    let root = PathBuf::from(&project_name);
    if root.exists() {
        return Err(format!("directory '{}' already exists", root.display()));
    }
    fs::create_dir_all(root.join("src")).map_err(|e| format!("create dir failed: {e}"))?;

    let pkg_name = project_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    let cargo_toml = format!(
        "[package]\nname = \"{pkg_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nrustui = \"0.2.0\"\n"
    );
    fs::write(root.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;
    fs::write(root.join(".gitignore"), "/target\nCargo.lock\n").map_err(|e| e.to_string())?;

    match template {
        TemplateKind::App => {
            let main_rs = format!(
                "use rustui::prelude::*;\n\n#[derive(Observable, Clone, Default)]\nstruct AppModel {{\n    count: i32,\n    status: String,\n}}\n\nstruct MainView;\n\nimpl View for MainView {{\n    fn body(self, cx: &mut Cx) -> impl View {{\n        let model = cx.model(AppModel {{\n            count: 0,\n            status: \"Ready\".into(),\n        }});\n\n        VStack::new(\n            12,\n            (\n                Text::new(\"{pkg_name}\").font(Font::title()),\n                Text::new(move || format!(\"Count: {{}} ({{}})\", model.count(), model.status()))\n                    .color(Color::secondary()),\n                HStack::new(\n                    8,\n                    (\n                        Button::new(\"Increment\", move || model.update_count(|c| *c += 1)),\n                        Button::new(\"Reset\", move || model.set_count(0))\n                            .style(ButtonStyle::secondary(&Theme::light())),\n                    ),\n                ),\n            ),\n        )\n        .padding(24)\n        .frame(Frame::new().max_width(f32::INFINITY).max_height(f32::INFINITY))\n    }}\n}}\n\n#[preview(\"Default\")]\nfn preview_main() -> AnyView {{\n    AnyView::new(MainView)\n}}\n\nfn main() {{\n    App::new(MainView)\n        .title(\"{pkg_name}\")\n        .size(720, 480)\n        .run();\n}}\n"
            );
            fs::write(root.join("src/main.rs"), main_rs).map_err(|e| e.to_string())?;
        }
        TemplateKind::MultiWindow => {
            let main_rs = format!(
                "use rustui::prelude::*;\n\nstruct WorkspaceView;\n\nimpl View for WorkspaceView {{\n    fn body(self, cx: &mut Cx) -> impl View {{\n        let split = cx.state(0.35f32);\n        VStack::new(\n            0,\n            (\n                Toolbar::new((\n                    Text::new(\"{pkg_name} Workspace\").font(Font::headline()),\n                    Spacer::new(),\n                    Text::new(\"RustUI 0.2 Multi-Window\").color(Color::secondary()),\n                )),\n                SplitView::horizontal(\n                    split.binding(),\n                    VStack::new(8, (Text::new(\"Navigator\").font(Font::headline()), Spacer::new()))\n                        .padding(12),\n                    VStack::new(8, (Text::new(\"Primary Canvas\").font(Font::title()), Spacer::new()))\n                        .padding(16),\n                ),\n                StatusBar::new((Text::new(\"Ready\"), Spacer::new(), Text::new(\"Window 1\"))),\n            ),\n        )\n    }}\n}}\n\nfn main() {{\n    App::new(WorkspaceView)\n        .title(\"{pkg_name} — Main\")\n        .size(960, 640)\n        .run();\n}}\n"
            );
            fs::write(root.join("src/main.rs"), main_rs).map_err(|e| e.to_string())?;
        }
        TemplateKind::Lib => {
            let lib_rs = "use rustui::prelude::*;\n\n/// Reusable card widget exported by this RustUI 0.2 library crate.\n#[derive(Debug, Clone)]\npub struct InfoCard {\n    pub title: String,\n    pub detail: String,\n}\n\nimpl InfoCard {\n    pub fn new(title: impl Into<String>, detail: impl Into<String>) -> Self {\n        Self {\n            title: title.into(),\n            detail: detail.into(),\n        }\n    }\n}\n\nimpl View for InfoCard {\n    fn body(self, _cx: &mut Cx) -> impl View {\n        VStack::new(\n            6,\n            (\n                Text::new(self.title).font(Font::headline()),\n                Text::new(self.detail).color(Color::secondary()),\n            ),\n        )\n        .padding(14)\n        .background(Color::surface())\n        .border(1, Color::border())\n        .corner_radius(8)\n    }\n}\n\n#[preview(\"InfoCard Default\")]\npub fn preview_info_card() -> AnyView {\n    AnyView::new(InfoCard::new(\"RustUI 0.2\", \"Reusable library component\"))\n}\n";
            fs::write(root.join("src/lib.rs"), lib_rs).map_err(|e| e.to_string())?;
        }
    }

    println!(
        "Created RustUI 0.2 project '{}' using {:?} template.",
        root.display(),
        template
    );
    Ok(())
}

fn collect_mtimes(dir: &Path, out: &mut HashMap<PathBuf, SystemTime>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name != "target" && !name.starts_with('.') {
                collect_mtimes(&p, out);
            }
        } else if let Ok(meta) = entry.metadata()
            && let Ok(mtime) = meta.modified()
        {
            out.insert(p, mtime);
        }
    }
}

fn spawn_cargo_run(
    example: Option<&str>,
    release: bool,
    hot_reload: bool,
) -> Result<Child, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run");
    if let Some(ex) = example {
        cmd.arg("--example").arg(ex);
    }
    if release {
        cmd.arg("--release");
    }
    if hot_reload {
        cmd.env("RUSTUI_HOT_RELOAD", "1");
    }
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to launch cargo run: {e}"))
}

fn cmd_dev(args: &[String]) -> Result<(), String> {
    let mut example: Option<String> = None;
    let mut once = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--example" => {
                i += 1;
                example = Some(
                    args.get(i)
                        .ok_or_else(|| "--example requires a target name".to_string())?
                        .clone(),
                );
            }
            "--once" => once = true,
            other => return Err(format!("unknown flag '{other}' for dev")),
        }
        i += 1;
    }

    let watch_root = PathBuf::from("src");
    let mut snapshot = HashMap::new();
    collect_mtimes(&watch_root, &mut snapshot);

    println!(
        "Starting RustUI dev session (RUSTUI_HOT_RELOAD=1, example={})...",
        example.as_deref().unwrap_or("<default>")
    );
    let mut child = spawn_cargo_run(example.as_deref(), false, true)?;
    if once {
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("cargo run exited with {status}"));
        }
        return Ok(());
    }

    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("dev process exited with {status}"));
            }
            break;
        }
        thread::sleep(Duration::from_millis(450));
        let mut latest = HashMap::new();
        collect_mtimes(&watch_root, &mut latest);
        if latest != snapshot {
            println!("Detected source changes under src/; restarting dev target...");
            snapshot = latest;
            let _ = child.kill();
            let _ = child.wait();
            child = spawn_cargo_run(example.as_deref(), false, true)?;
        }
    }
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run");
    for arg in args {
        cmd.arg(arg);
    }
    let status = cmd.status().map_err(|e| format!("cargo run failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo run exited with {status}"))
    }
}

fn cmd_build(args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build");
    for arg in args {
        cmd.arg(arg);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("cargo build failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo build exited with {status}"))
    }
}

fn probe_tool(program: &str, arg: &str) -> String {
    match Command::new(program).arg(arg).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(out) => format!(
            "exit {} ({})",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("unavailable ({e})"),
    }
}

fn cmd_doctor() -> Result<(), String> {
    let caps = rustui_platform::detect_platform_capabilities();
    let theme = rustui_platform::detect_system_theme();
    let reduced_motion = rustui_platform::detect_reduced_motion();
    println!("RustUI 0.2 Environment Diagnostics");
    println!("  rustc         : {}", probe_tool("rustc", "--version"));
    println!("  cargo         : {}", probe_tool("cargo", "--version"));
    println!(
        "  rust-analyzer : {}",
        probe_tool("rust-analyzer", "--version")
    );
    println!(
        "  cargo-deny    : {}",
        probe_tool("cargo-deny", "--version")
    );
    println!("  os            : {}", caps.os);
    println!("  arch          : {}", std::env::consts::ARCH);
    println!("  display_server: {}", caps.display_server);
    println!("  ime_backend   : {}", caps.ime_backend);
    println!("  accessibility : {}", caps.accessibility_backend);
    println!(
        "  wayland       : {}",
        env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".into())
    );
    println!(
        "  x11           : {}",
        env::var("DISPLAY").unwrap_or_else(|_| "<unset>".into())
    );
    println!("  system_theme  : {:?}", theme);
    println!("  reduced_motion: {reduced_motion}");
    println!("  capabilities  : {caps:?}");
    Ok(())
}

fn cmd_bench() -> Result<(), String> {
    println!("Running RustUI 0.2 performance target suite...");
    let status = Command::new("cargo")
        .args([
            "test",
            "-p",
            "rustui",
            "--test",
            "perf_targets",
            "--",
            "--nocapture",
        ])
        .status()
        .map_err(|e| format!("failed to run perf_targets: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("perf_targets exited with {status}"))
    }
}

fn cmd_bundle(args: &[String]) -> Result<(), String> {
    let mut format = BundleFormat::AppImage;
    let mut name: Option<String> = None;
    let mut skip_build = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                i += 1;
                let s = args.get(i).ok_or_else(|| {
                    "--format requires app, appimage, flatpak, or msix".to_string()
                })?;
                format = BundleFormat::from_str(s)?;
            }
            "--name" => {
                i += 1;
                name = Some(
                    args.get(i)
                        .ok_or_else(|| "--name requires an application name".to_string())?
                        .clone(),
                );
            }
            "--no-build" => skip_build = true,
            other => return Err(format!("unknown bundle flag '{other}'")),
        }
        i += 1;
    }

    let cwd_name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "rustui-app".into());
    let app_name = name.unwrap_or(cwd_name);

    if !skip_build {
        let status = Command::new("cargo")
            .args(["build", "--release"])
            .status()
            .map_err(|e| format!("cargo build --release failed: {e}"))?;
        if !status.success() {
            return Err(format!("release build failed with {status}"));
        }
    }

    let stage_root = PathBuf::from("target").join("bundle").join(format.as_str());
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;

    match format {
        BundleFormat::App => {
            let bundle = stage_root.join(format!("{app_name}.app/Contents"));
            fs::create_dir_all(bundle.join("MacOS")).map_err(|e| e.to_string())?;
            fs::create_dir_all(bundle.join("Resources")).map_err(|e| e.to_string())?;
            let plist = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\n  <key>CFBundleName</key><string>{app_name}</string>\n  <key>CFBundleExecutable</key><string>{app_name}</string>\n  <key>CFBundlePackageType</key><string>APPL</string>\n</dict></plist>\n"
            );
            fs::write(bundle.join("Info.plist"), plist).map_err(|e| e.to_string())?;
        }
        BundleFormat::AppImage => {
            let appdir = stage_root.join(format!("{app_name}.AppDir/usr/bin"));
            fs::create_dir_all(&appdir).map_err(|e| e.to_string())?;
            fs::write(
                stage_root.join(format!("{app_name}.AppDir/{app_name}.desktop")),
                format!("[Desktop Entry]\nType=Application\nName={app_name}\nExec={app_name}\nCategories=Development;\n"),
            )
            .map_err(|e| e.to_string())?;
            fs::write(
                stage_root.join(format!("{app_name}.AppDir/AppRun")),
                format!("#!/bin/sh\nexec \"$(dirname \"$0\")/usr/bin/{app_name}\" \"$@\"\n"),
            )
            .map_err(|e| e.to_string())?;
        }
        BundleFormat::Flatpak => {
            let manifest = format!(
                "{{\n  \"app-id\": \"dev.rustui.{app_name}\",\n  \"runtime\": \"org.freedesktop.Platform\",\n  \"runtime-version\": \"24.08\",\n  \"sdk\": \"org.freedesktop.Sdk\",\n  \"command\": \"{app_name}\"\n}}\n"
            );
            fs::write(
                stage_root.join(format!("dev.rustui.{app_name}.json")),
                manifest,
            )
            .map_err(|e| e.to_string())?;
        }
        BundleFormat::Msix => {
            let appx = format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Package xmlns=\"http://schemas.microsoft.com/appx/manifest/foundation/windows10\">\n  <Identity Name=\"dev.rustui.{app_name}\" Publisher=\"CN=RustUI\" Version=\"0.2.0.0\" />\n  <Properties><DisplayName>{app_name}</DisplayName></Properties>\n</Package>\n"
            );
            fs::write(stage_root.join("AppxManifest.xml"), appx).map_err(|e| e.to_string())?;
        }
    }

    println!(
        "Staged {} bundle for '{}' at {}",
        format.as_str(),
        app_name,
        stage_root.display()
    );
    Ok(())
}

fn run_cli() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print_usage();
        return Ok(());
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "new" => cmd_new(rest),
        "dev" => cmd_dev(rest),
        "run" => cmd_run(rest),
        "build" => cmd_build(rest),
        "doctor" => cmd_doctor(),
        "bench" => cmd_bench(),
        "bundle" => cmd_bundle(rest),
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!(
            "unknown subcommand '{other}'. Try `rustui --help`."
        )),
    }
}

fn main() -> ExitCode {
    if let Err(err) = run_cli() {
        eprintln!("rustui error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
