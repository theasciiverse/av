//! av, the Asciiverse terminal client.

mod app;
mod cast;
mod catalog;
mod install;
mod launch;
mod md;
mod paths;
mod state;
mod theme;
mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use ratatui::crossterm::execute;

use app::App;

fn main() -> Result<()> {
    let paths = paths::Paths::new()?;
    init_logging(&paths);
    let url = std::env::var("AV_CATALOG_URL").unwrap_or_else(|_| catalog::DEFAULT_CATALOG_URL.into());
    log::info!("av {} starting, catalog {url}", env!("CARGO_PKG_VERSION"));

    let mut app = App::new(paths, url);
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let res = run(&mut terminal, &mut app);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

/// Session logging per the Asciiverse convention: with RUST_LOG set, every
/// key, action and state change goes to av.log in the cache dir.
fn init_logging(paths: &paths::Paths) {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let file = paths.cache.join("av.log");
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        env_logger::Builder::from_default_env().target(env_logger::Target::Pipe(Box::new(f))).init();
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut last_snapshot = std::time::Instant::now();
    while !app.quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        // wait for input up to one frame, then take everything queued, so a
        // fast wheel spin is handled at once instead of one notch per frame
        let mut wait = Duration::from_millis(33);
        while event::poll(wait)? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
            if app.launch.is_some() || app.quit {
                break;
            }
            wait = Duration::ZERO;
        }
        if let Some(t) = app.launch.take() {
            // the title gets a plain terminal; it turns on its own mouse mode if it wants one
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            ratatui::restore();
            launch(app, &t);
            *terminal = ratatui::init();
            let _ = execute!(std::io::stdout(), EnableMouseCapture);
            terminal.clear()?;
        }
        if last_snapshot.elapsed() > Duration::from_millis(500) {
            last_snapshot = std::time::Instant::now();
            log::trace!(
                "snapshot view={:?} filter={:?} selected={} focus={:?} installs={}",
                app.view, app.filter, app.selected, app.focus, app.progress.len()
            );
        }
    }
    Ok(())
}

/// Run a title as a separate process in the same terminal; av never links it.
fn launch(app: &mut App, t: &catalog::Title) {
    let cmd = if let Some(c) = &t.connect {
        log::info!("connect {} {}:{}", t.id, c.host, c.port);
        let mut cmd = std::process::Command::new("telnet");
        cmd.arg(&c.host).arg(c.port.to_string());
        cmd
    } else {
        let dir = app.paths.title_dir(&t.kind, &t.id);
        let Some(exe) = install::find_exec(&dir, &t.exec_name()) else {
            app.status = format!("{} is not installed correctly; reinstall it", t.title);
            return;
        };
        log::info!("launch {} {}", t.id, exe.display());
        let mut cmd = std::process::Command::new(&exe);
        cmd.current_dir(&dir);
        cmd
    };
    let log = app.paths.cache.join("logs").join(format!("{}.log", t.id));
    let outcome = launch::run(cmd, Some(log));
    app.finished(t, outcome);
}
