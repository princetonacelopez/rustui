//! `rustui` developer CLI: `new`, `run`, and `doctor`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser, Subcommand};
use rustui::platform::detect_platform_capabilities;
use rustui::render::probe_gpu_adapter;
use rustui::text::TextSystem;

#[derive(Parser)]
#[command(
    name = "rustui",
    version,
    about = "CLI for building, running, and diagnosing RustUI desktop applications"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new RustUI project from the starter template.
    New {
        /// Project directory and crate name.
        name: String,
    },
    /// Run a RustUI application or built-in example.
    Run {
        /// Optional example name (counter, todo, text_editor, dashboard, terminal).
        #[arg(long)]
        example: Option<String>,
        /// Run headlessly without opening a desktop window.
        #[arg(long)]
        headless: bool,
        /// Enable the debug inspector overlay.
        #[arg(long)]
        inspector: bool,
        /// Save a PNG snapshot when running headlessly.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Inspect GPU drivers, windowing stack, fonts, IME, and accessibility dependencies.
    Doctor,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Run {
            example,
            headless,
            inspector,
            snapshot,
        } => cmd_run(example, headless, inspector, snapshot),
        Commands::Doctor => cmd_doctor(),
    }
}

fn cmd_new(name: &str) {
    let dir = PathBuf::from(name);
    if dir.exists() {
        eprintln!("Error: destination '{}' already exists.", dir.display());
        std::process::exit(1);
    }
    let src_dir = dir.join("src");
    if let Err(e) = fs::create_dir_all(&src_dir) {
        eprintln!("Failed to create directory: {e}");
        std::process::exit(1);
    }

    let pkg_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or(name);

    let cargo_toml = format!(
        r#"[package]
name = "{pkg_name}"
version = "0.1.0"
edition = "2024"

[dependencies]
rustui = "0.1"
"#
    );

    let main_rs = r#"use rustui::prelude::*;

struct MyApp;

impl View for MyApp {
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

fn main() -> Result<(), String> {
    App::new(MyApp)
        .title("My RustUI App")
        .size(480.0, 320.0)
        .run()
}
"#;

    let gitignore = "/target\n";
    let readme = format!("# {name}\n\nCreated with `rustui new`.\n\n```bash\ncargo run\n```\n");

    let writes = [
        (dir.join("Cargo.toml"), cargo_toml),
        (src_dir.join("main.rs"), main_rs.to_owned()),
        (dir.join(".gitignore"), gitignore.to_owned()),
        (dir.join("README.md"), readme),
    ];
    for (path, content) in writes {
        if let Err(e) = fs::write(&path, content) {
            eprintln!("Failed to write {}: {e}", path.display());
            std::process::exit(1);
        }
    }

    println!("Created RustUI project in '{}'.", dir.display());
    println!("Run with:\n  cd {name} && cargo run");
}

fn cmd_run(example: Option<String>, headless: bool, inspector: bool, snapshot: Option<PathBuf>) {
    let mut cmd = Command::new("cargo");
    cmd.arg("run");
    if let Some(ex) = example {
        cmd.args(["-p", "rustui", "--example", &ex]);
    }
    cmd.arg("--");
    if headless {
        cmd.arg("--headless");
    }
    if inspector {
        cmd.arg("--inspector");
    }
    if let Some(snap) = snapshot {
        cmd.arg("--snapshot").arg(snap);
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("Failed to invoke cargo run: {e}");
        std::process::exit(1);
    });
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn cmd_doctor() {
    let gpu = probe_gpu_adapter();
    let plat = detect_platform_capabilities();
    let mut text_sys = TextSystem::new();
    let font_families = text_sys.discovered_font_families();

    println!("RustUI Doctor Report");
    println!("====================");
    println!(
        "OS:                  {} ({})",
        plat.os,
        std::env::consts::ARCH
    );
    println!("Display server:      {}", plat.display_server);
    if plat.os == "Linux" {
        println!("Linux priority:      {}", plat.preferred_linux_path);
    }
    println!(
        "GPU adapter:         {} (backend: {}, type: {})",
        gpu.adapter_name, gpu.backend, gpu.device_type
    );
    println!(
        "Vello compute:       {}",
        if gpu.vello_supported {
            "available"
        } else {
            "software fallback"
        }
    );
    println!("Discovered fonts:    {} families", font_families);
    println!("IME backend:         {}", plat.ime_backend);
    println!("Accessibility:       {}", plat.accessibility_backend);
    println!(
        "Clipboard:           {}",
        if plat.clipboard_ready {
            "OS clipboard ready"
        } else {
            "in-memory fallback (headless)"
        }
    );
}
