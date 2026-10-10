//! av, the Asciiverse terminal client.

mod app;
mod cast;
mod catalog;
mod install;
mod md;
mod paths;
mod state;
mod theme;
mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;

fn main() -> Result<()> {
    let paths = paths::Paths::new()?;
    init_logging(&paths);
    let url = std::env::var("AV_CATALOG_URL").unwrap_or_else(|_| catalog::DEFAULT_CATALOG_URL.into());
    log::info!("av {} starting, catalog {url}", env!("CARGO_PKG_VERSION"));

    let mut app = App::new(paths, url);
    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app);
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
        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    app.on_key(k);
                }
            }
        }
        if let Some(t) = app.launch.take() {
            ratatui::restore();
            launch(app, &t);
            *terminal = ratatui::init();
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
    let status = if let Some(c) = &t.connect {
        log::info!("connect {} {}:{}", t.id, c.host, c.port);
        std::process::Command::new("telnet").arg(&c.host).arg(c.port.to_string()).status()
    } else {
        let dir = app.paths.title_dir(&t.kind, &t.id);
        let Some(exe) = install::find_exec(&dir, &t.exec_name()) else {
            app.status = format!("{} is not installed correctly; reinstall it", t.title);
            return;
        };
        log::info!("launch {} {}", t.id, exe.display());
        std::process::Command::new(&exe).current_dir(&dir).status()
    };
    app.status = match status {
        Ok(s) if s.success() => format!("back from {}", t.title),
        Ok(s) => format!("{} exited with {s}", t.title),
        Err(e) if t.connect.is_some() && e.kind() == std::io::ErrorKind::NotFound => {
            "telnet is not installed; Lera will replace it".into()
        }
        Err(e) => format!("could not start {}: {e}", t.title),
    };
    log::info!("{}", app.status);
}
