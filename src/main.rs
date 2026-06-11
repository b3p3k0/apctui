// SPDX-License-Identifier: GPL-3.0-or-later
//! apctui — TUI monitor & manager for apcupsd (multi-UPS).

use apctui::{app, nis, poller, privileged, registry, theme, ui};

use anyhow::Result;
use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Version with build identity: `0.5.0 (g039c121)` — the hash answers
/// "is this binary current" definitively, unlike the bare semver.
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (g", env!("APCTUI_GIT_HASH"), ")");

#[derive(Parser)]
#[command(version = VERSION, about = "Slick TUI monitor & manager for apcupsd", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// UPS to monitor as NAME=HOST:PORT (repeatable, overrides config file)
    #[arg(long, value_name = "NAME=HOST:PORT", global = true)]
    ups: Vec<String>,

    /// Path to apctui config file (default: ~/.config/apctui/config.toml)
    #[arg(long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    /// Plain-monitor mode: ASCII glyphs, no colors or gradients
    #[arg(long)]
    basic: bool,

    /// Poll interval in seconds
    #[arg(long, default_value_t = 2.0)]
    interval: f64,

    /// Fetch each UPS status once, print it, and exit (no TUI)
    #[arg(long)]
    probe: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Privileged helper: validate, back up, atomically write a config, and
    /// optionally restart the daemon. Invoked via pkexec/sudo by the TUI.
    Apply {
        #[arg(long)]
        dest: PathBuf,
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        restart: Option<String>,
        #[arg(long)]
        service: Option<String>,
    },
    /// Privileged helper: perform a single systemctl action on an instance.
    Service {
        #[arg(long)]
        action: String,
        #[arg(long)]
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Privileged subcommands run without a TUI and exit. Print a clean
    // one-line error rather than an anyhow backtrace — the TUI parses stderr.
    match &cli.command {
        Some(Cmd::Apply { dest, src, restart, service }) => {
            if let Err(e) = privileged::apply_main(dest, src, restart.as_deref(), service.as_deref()) {
                eprintln!("Error: {e:#}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(Cmd::Service { action, name }) => {
            if let Err(e) = privileged::service_main(action, name) {
                eprintln!("Error: {e:#}");
                std::process::exit(1);
            }
            return Ok(());
        }
        None => {}
    }

    let (upses, source) = registry::resolve_with_source(&cli.ups, cli.config.as_deref())?;

    if cli.probe {
        println!("[{}]", source.describe(upses.len()));
        return probe(&upses);
    }

    let interval = Duration::from_secs_f64(cli.interval.max(0.2));
    let (tx, rx) = mpsc::channel();
    for (i, u) in upses.iter().enumerate() {
        poller::spawn(i, u.addr.clone(), interval, tx.clone());
    }

    let color_mode = if cli.basic { theme::ColorMode::Mono } else { theme::detect() };
    let notify_opts = apctui::options::load();
    let mut app = app::App::new(&upses, cli.basic, notify_opts);
    app.notify_info(source.describe(upses.len()));

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &rx, color_mode);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut app::App,
    rx: &mpsc::Receiver<poller::Update>,
    color_mode: theme::ColorMode,
) -> Result<()> {
    loop {
        while let Ok(update) = rx.try_recv() {
            app.apply(update);
        }
        app.tick();
        let mode = if app.basic { theme::ColorMode::Mono } else { color_mode };
        let theme = theme::Theme::new(mode, app.basic);
        terminal.draw(|f| ui::draw(f, app, &theme))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key.code, key.modifiers);
                }
            }
        }
        if app.should_quit {
            return Ok(());
        }
    }
}

fn probe(upses: &[registry::UpsRef]) -> Result<()> {
    let mut failures = 0;
    for u in upses {
        println!("=== {} ({}) ===", u.name, u.addr);
        match nis::fetch_status(&u.addr, Duration::from_secs(3)) {
            Ok(s) => {
                let mut keys: Vec<_> = s.fields.keys().collect();
                keys.sort();
                for k in keys {
                    println!("{k:<10} {}", s.fields[k]);
                }
            }
            Err(e) => {
                failures += 1;
                println!("error: {e:#}");
            }
        }
        println!();
    }
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}
