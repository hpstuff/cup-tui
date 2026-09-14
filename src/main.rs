mod api;
mod app;
mod cup;
mod keys;
mod model;
mod text;
mod ui;

use std::io::{self, Stdout, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use app::{App, Effect, Msg};
use cup::CupClient;

type Term = Terminal<CrosstermBackend<Stdout>>;

struct Args {
    profile: Option<String>,
    cup_bin: String,
}

fn parse_args() -> Result<Args> {
    let mut profile = None;
    let mut cup_bin = std::env::var("CUP_BIN").unwrap_or_else(|_| "cup".into());
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--profile" => profile = Some(it.next().context("--profile needs a name")?),
            "--cup" => cup_bin = it.next().context("--cup needs a path")?,
            "-h" | "--help" => {
                println!(
                    "cup-tui — fullscreen ClickUp TUI on top of the cup CLI\n\n\
                     USAGE: cup-tui [-p PROFILE] [--cup PATH]\n\n\
                     -p, --profile NAME   use a cup profile (passed as `cup -p NAME`)\n\
                     --cup PATH           cup binary to use (default: cup, or $CUP_BIN)\n\
                     -h, --help           this help\n\
                     -V, --version        version\n\n\
                     Press ? inside the app for keys. $VISUAL / $EDITOR is used for comments and descriptions."
                );
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("cup-tui {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(Args { profile, cup_bin })
}

fn enter_tui() -> Result<Term> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    term.clear()?;
    term.hide_cursor()?;
    Ok(term)
}

fn leave_tui() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = io::stdout().flush();
}

/// Reads crossterm events on a plain thread. While `paused` is set it does not
/// touch stdin, so an external editor can own the terminal.
fn spawn_input_thread(tx: mpsc::UnboundedSender<Msg>, paused: Arc<AtomicBool>, idle: Arc<AtomicBool>) {
    std::thread::spawn(move || loop {
        if paused.load(Ordering::SeqCst) {
            idle.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        idle.store(false, Ordering::SeqCst);
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => {
                if paused.load(Ordering::SeqCst) {
                    continue;
                }
                match event::read() {
                    Ok(ev) => {
                        if tx.send(Msg::Term(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            Ok(false) => {}
            Err(_) => break,
        }
    });
}

/// Runs $VISUAL / $EDITOR on a temp file seeded with `initial`.
/// Returns (text if changed and editor exited 0, path of the temp file).
fn run_editor(initial: &str, suffix: &str) -> Result<(Option<String>, PathBuf)> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".into());
    let file = tempfile::Builder::new()
        .prefix("cup-tui-")
        .suffix(suffix)
        .tempfile()?;
    let (mut handle, path) = file.keep()?;
    handle.write_all(initial.as_bytes())?;
    handle.flush()?;
    drop(handle);

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()
        .with_context(|| format!("failed to launch editor `{editor}`"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Ok((None, path));
    }
    let text = std::fs::read_to_string(&path)?;
    if text == initial {
        let _ = std::fs::remove_file(&path);
        return Ok((None, path));
    }
    Ok((Some(text), path))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    // Fail fast with a readable message if cup is missing.
    if let Err(e) = std::process::Command::new(&args.cup_bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        anyhow::bail!(
            "cannot run `{}`: {e}\nInstall the cup CLI (and run `cup init`) or pass --cup PATH.",
            args.cup_bin
        );
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let api = api::ApiClient::from_cup_config(args.profile.as_deref());
    let client = Arc::new(CupClient::new(args.cup_bin, args.profile));
    let mut app = App::new(client, api, tx.clone());
    app.init();

    let paused = Arc::new(AtomicBool::new(false));
    let idle = Arc::new(AtomicBool::new(false));
    spawn_input_thread(tx.clone(), paused.clone(), idle.clone());

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(100));
            loop {
                iv.tick().await;
                if tx.send(Msg::Tick).is_err() {
                    break;
                }
            }
        });
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_tui();
        default_hook(info);
    }));

    let mut terminal = enter_tui()?;
    terminal.draw(|f| ui::draw(f, &mut app))?;

    while let Some(msg) = rx.recv().await {
        let mut redraw = app.update(msg);
        // coalesce anything else already queued before drawing
        while let Ok(m) = rx.try_recv() {
            redraw |= app.update(m);
            if app.should_quit {
                break;
            }
        }

        for eff in app.take_effects() {
            match eff {
                Effect::Editor { purpose, initial } => {
                    paused.store(true, Ordering::SeqCst);
                    let wait = std::time::Instant::now();
                    while !idle.load(Ordering::SeqCst) && wait.elapsed() < Duration::from_millis(500) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    leave_tui();
                    let result = run_editor(&initial, ".md");
                    terminal = enter_tui()?;
                    paused.store(false, Ordering::SeqCst);
                    match result {
                        Ok((text, path)) => app.on_editor_result(purpose, text, Some(path)),
                        Err(e) => app.toast(format!("editor: {e}"), app::ToastKind::Error),
                    }
                    redraw = true;
                }
            }
        }

        if app.should_quit {
            break;
        }
        if redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
        }
    }

    leave_tui();
    Ok(())
}
