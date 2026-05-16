mod app;
mod player;
mod resolve;
mod search;
mod types;
mod ui;
#[cfg(windows)]
mod winjob;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::player::Player;

fn main() -> Result<()> {
    install_panic_hook();
    if let Err(e) = run() {
        let _ = restore_terminal();
        eprintln!("{}: {e:#}", env!("CARGO_PKG_NAME"));
        std::process::exit(1);
    }
    Ok(())
}

fn run() -> Result<()> {
    // Tie every child we spawn (mpv, yt-dlp) to our own lifetime via a
    // kill-on-job-close job object on Windows. With this, closing the
    // terminal — or `taskkill /f`, or a panic — terminates mpv AND any
    // in-flight yt-dlp instead of leaving them as orphans. Held until run()
    // returns; the OS closes the handle for us on any unclean exit.
    #[cfg(windows)]
    let _job = match winjob::KillOnExit::install() {
        Ok(j) => {
            log_startup("kill-on-close job installed OK");
            Some(j)
        }
        Err(e) => {
            log_startup(&format!("kill-on-close install FAILED: {e:#}"));
            None
        }
    };
    // Secondary mechanism — independent of the job object. If the terminal
    // closes (or Ctrl-C / log-off / shutdown), this handler runs in our
    // process and kills mpv directly via TerminateProcess. Covers cases
    // where the job-object route silently doesn't work.
    #[cfg(windows)]
    match winjob::install_ctrl_handler() {
        Ok(()) => log_startup("console ctrl handler installed OK"),
        Err(e) => log_startup(&format!("console ctrl handler install FAILED: {e:#}")),
    }

    let (player_tx, player_rx) = channel();
    let player = Player::spawn(player_tx).context("starting mpv")?;
    let (search_tx, search_rx) = channel();
    let (resolve_tx, resolve_rx) = channel();

    let mut app = App::new(player, search_tx, resolve_tx);

    let mut terminal = setup_terminal().context("terminal setup")?;
    let result = event_loop(&mut terminal, &mut app, player_rx, search_rx, resolve_rx);
    let _ = restore_terminal();
    // App::Drop saves the playlist; Player::Drop sends mpv quit and reaps it.
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    player_rx: std::sync::mpsc::Receiver<player::PlayerEvent>,
    search_rx: std::sync::mpsc::Receiver<search::SearchEvent>,
    resolve_rx: std::sync::mpsc::Receiver<resolve::ResolveEvent>,
) -> Result<()> {
    let tick = Duration::from_millis(50);
    let mut last = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        drain(&player_rx, |ev| app.on_player_event(ev));
        drain(&search_rx, |ev| app.on_search_event(ev));
        drain(&resolve_rx, |ev| app.on_resolve_event(ev));

        let remaining = tick.saturating_sub(last.elapsed());
        if event::poll(remaining)? {
            if let Event::Key(k) = event::read()? {
                app.on_key(k);
            }
        }

        if last.elapsed() >= tick {
            last = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn drain<T>(rx: &std::sync::mpsc::Receiver<T>, mut on: impl FnMut(T)) {
    while let Ok(v) = rx.try_recv() {
        on(v);
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal() -> Result<()> {
    let mut out = stdout();
    execute!(out, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        prev(info);
    }));
}

fn log_startup(msg: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("{}-debug.log", env!("CARGO_PKG_NAME")));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] pid={} {msg}", std::process::id());
    }
}
