//! App state and input. Arrow keys, Tab between panels, Enter selects, Esc
//! backs out one step; Esc at the top level asks before quitting.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::cast::{Cast, Player};
use crate::catalog::{self, Title};
use serde::{Deserialize, Serialize};
use crate::install::{self, Progress};
use crate::paths::Paths;
use crate::state::State;

/// What the list shows, set in the card under the search box and kept in
/// the config dir. It stores the kinds switched off, so a kind that is new
/// to the catalog shows up. `installed` and `wishlist` narrow the list to
/// the player's own titles: either one, or both together (installed or
/// wishlisted).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Show {
    pub hidden: BTreeSet<String>,
    pub installed: bool,
    pub wishlist: bool,
}

impl Show {
    fn file(config: &Path) -> std::path::PathBuf {
        config.join("show.json")
    }

    pub fn load(config: &Path) -> Show {
        crate::state::read(&Show::file(config))
    }

    fn save(&self, config: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(config)?;
        crate::state::write_atomic(&Show::file(config), &serde_json::to_string_pretty(self)?)
    }

    pub fn lets_through(&self, t: &Title, state: &State) -> bool {
        !self.hidden.contains(&t.kind)
            && (!(self.installed || self.wishlist)
                || (self.installed && state.installed.contains_key(&t.id))
                || (self.wishlist && state.wishlist.contains(&t.id)))
    }
}

/// A line in the show card.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShowRow {
    All,
    Kind(String),
    Installed,
    Wishlist,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    /// the show card, open
    Show,
    List,
    Actions,
    Page,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Install,
    Uninstall,
    Play,
    Wishlist,
}

pub enum Modal {
    Quit,
    Uninstall { id: String },
    Message(String),
    /// a question with buttons, e.g. before launching
    Choice { title: String, text: String, options: Vec<(String, Then)>, sel: usize },
}

/// What a Choice button does.
#[derive(Clone, Debug)]
pub enum Then {
    Cancel,
    /// go on with the launch checks; size_ok once the size warning was accepted
    Play { id: String, size_ok: bool },
    /// launch now, all checks done
    Launch(String),
    Install(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Installed,
    Update,
    Busy,
}

/// What a screen area does when clicked; ui::draw records these every frame.
#[derive(Clone, Copy, Debug)]
pub enum Hit {
    /// a title row, by index into `visible()`
    Row(usize),
    /// the show card's folded line, which opens and closes it
    ShowHead,
    /// a line in the open show card, by index into `show_rows()`
    ShowRow(usize),
    /// an action button, by index into `actions()`
    Button(usize),
    List,
    Search,
    Detail,
}

pub enum CastMsg {
    Loaded(String, Result<Cast, String>),
}

pub struct App {
    pub paths: Paths,
    pub state: State,
    pub titles: Vec<Title>,
    pub collections: catalog::Collections,
    pub catalog_url: String,
    pub offline: bool,
    pub show: Show,
    /// the highlighted line in the open show card
    pub show_row: usize,
    pub filter: String,
    pub selected: usize,
    /// the list's scroll position, in rows, kept between frames so the list
    /// only moves when the selection would leave the view
    pub list_offset: usize,
    pub focus: Focus,
    pub action: usize,
    pub page_scroll: u16,
    pub modal: Option<Modal>,
    pub uninstall_keep_data: bool,
    pub status: String,
    pub progress: HashMap<String, (u64, u64, String)>,
    pub quit: bool,
    /// set when a title should be launched; main suspends the TUI for it
    pub launch: Option<Title>,
    pub player: Option<(String, Player)>,
    pub hits: Vec<(Rect, Hit)>,
    cast_loading: HashSet<String>,
    cast_failed: HashMap<String, String>,
    tx: Sender<Progress>,
    rx: Receiver<Progress>,
    cast_tx: Sender<CastMsg>,
    cast_rx: Receiver<CastMsg>,
    last_tick: Instant,
}

impl App {
    pub fn new(paths: Paths, catalog_url: String) -> Self {
        let state = State::load(&paths.state);
        let show = Show::load(&paths.config);
        let (tx, rx) = channel();
        let (cast_tx, cast_rx) = channel();
        let mut app = App {
            paths,
            state,
            titles: Vec::new(),
            collections: Default::default(),
            catalog_url,
            offline: false,
            show,
            show_row: 0,
            filter: String::new(),
            selected: 0,
            list_offset: 0,
            focus: Focus::List,
            action: 0,
            page_scroll: 0,
            modal: None,
            uninstall_keep_data: true,
            status: String::new(),
            progress: HashMap::new(),
            quit: false,
            launch: None,
            player: None,
            hits: Vec::new(),
            cast_loading: HashSet::new(),
            cast_failed: HashMap::new(),
            tx,
            rx,
            cast_tx,
            cast_rx,
            last_tick: Instant::now(),
        };
        app.refresh(false);
        app
    }

    pub fn refresh(&mut self, force: bool) {
        match catalog::load(&self.catalog_url, &self.paths.cache, force) {
            Ok(l) => {
                let mut titles: Vec<Title> = l.catalog.titles.into_values().collect();
                // grouped by kind (catalog::kind_rank), alphabetical within each
                let rank = catalog::kind_rank;
                titles.sort_by(|a, b| (rank(&a.kind), a.title.to_lowercase()).cmp(&(rank(&b.kind), b.title.to_lowercase())));
                self.titles = titles;
                self.collections = l.catalog.collections;
                self.offline = l.offline;
                if l.offline {
                    // for this run only: show.json keeps the player's own choice
                    self.show.installed = true;
                    self.status = "offline: showing the cached catalog".into();
                } else {
                    self.status = format!("{} titles", self.titles.len());
                }
            }
            Err(e) => {
                log::error!("catalog: {e:#}");
                self.status = format!("{e:#}");
                self.modal = Some(Modal::Message(format!("Could not load the catalog.\n\n{e:#}")));
            }
        }
        self.clamp();
    }

    pub fn visible(&self) -> Vec<&Title> {
        let f = self.filter.to_lowercase();
        let mut v: Vec<&Title> = self.titles
            .iter()
            .filter(|t| self.show.lets_through(t, &self.state))
            .filter(|t| t.matches(&f, &self.collections))
            .collect();
        if !f.is_empty() {
            // within each kind's group, titles whose name matches come first;
            // sort_by_key is stable, so the alphabetical order is kept inside
            let rank = catalog::kind_rank;
            v.sort_by_key(|t| (rank(&t.kind), !t.title.to_lowercase().contains(&f)));
        }
        v
    }

    /// After the search changes: the first title whose name matches, wherever
    /// its kind's group sits, or the top of the list. Searching "tarts" lands
    /// on tarts, not on a story whose description says "starts".
    fn name_match(&self) -> usize {
        let f = self.filter.to_lowercase();
        let v = self.visible();
        v.iter()
            .position(|t| t.title.to_lowercase() == f)
            .or_else(|| v.iter().position(|t| t.title.to_lowercase().starts_with(&f)))
            .or_else(|| v.iter().position(|t| t.title.to_lowercase().contains(&f)))
            .unwrap_or(0)
    }

    pub fn current(&self) -> Option<Title> {
        self.visible().get(self.selected).map(|t| (*t).clone())
    }

    pub fn status_of(&self, t: &Title) -> Status {
        if self.progress.contains_key(&t.id) {
            return Status::Busy;
        }
        match self.state.installed.get(&t.id) {
            None => Status::NotInstalled,
            Some(i) if !t.version.is_empty() && i.version != t.version => Status::Update,
            Some(_) => Status::Installed,
        }
    }

    pub fn actions(&self, t: &Title) -> Vec<(Action, String, bool)> {
        let wish = if self.state.wishlist.contains(&t.id) { "♥ Wishlisted" } else { "♡ Wishlist" };
        if t.is_mud() {
            return vec![(Action::Play, "Connect".into(), true), (Action::Wishlist, wish.into(), true)];
        }
        let st = self.status_of(t);
        let first = match st {
            Status::NotInstalled => (Action::Install, "Install".into(), catalog::pick_platform(t).is_some()),
            Status::Update => (Action::Install, "Update".into(), true),
            Status::Busy => (Action::Install, "Installing…".into(), false),
            Status::Installed => (Action::Uninstall, "Uninstall".into(), true),
        };
        let can_play = matches!(st, Status::Installed | Status::Update);
        let mut v = vec![first, (Action::Play, "Play".into(), can_play), (Action::Wishlist, wish.into(), true)];
        if st == Status::Update {
            v.insert(2, (Action::Uninstall, "Uninstall".into(), true));
        }
        v
    }

    /// The show card's lines: All, each kind the catalog has (in list order),
    /// then the two that narrow to the player's own titles.
    pub fn show_rows(&self) -> Vec<ShowRow> {
        let mut kinds: Vec<&str> = Vec::new();
        for t in &self.titles {
            if kinds.last() != Some(&t.kind.as_str()) && !kinds.contains(&t.kind.as_str()) {
                kinds.push(&t.kind);
            }
        }
        let mut rows = vec![ShowRow::All];
        rows.extend(kinds.into_iter().map(|k| ShowRow::Kind(k.to_string())));
        rows.extend([ShowRow::Installed, ShowRow::Wishlist]);
        rows
    }

    pub fn row_on(&self, r: &ShowRow) -> bool {
        match r {
            ShowRow::All => self.show.hidden.is_empty(),
            ShowRow::Kind(k) => !self.show.hidden.contains(k),
            ShowRow::Installed => self.show.installed,
            ShowRow::Wishlist => self.show.wishlist,
        }
    }

    /// Switch a show card line; the highlighted title stays selected when it
    /// is still in the list.
    fn toggle_show(&mut self, i: usize) {
        let Some(r) = self.show_rows().get(i).cloned() else { return };
        let keep = self.current().map(|t| t.id);
        match &r {
            // all on, or when they already are, all off
            ShowRow::All if self.show.hidden.is_empty() => {
                self.show.hidden = self.titles.iter().map(|t| t.kind.clone()).collect();
            }
            ShowRow::All => self.show.hidden.clear(),
            ShowRow::Kind(k) => {
                if !self.show.hidden.remove(k) {
                    self.show.hidden.insert(k.clone());
                }
            }
            ShowRow::Installed => self.show.installed = !self.show.installed,
            ShowRow::Wishlist => self.show.wishlist = !self.show.wishlist,
        }
        log::info!("show {r:?} -> {:?}", self.show);
        if let Err(e) = self.show.save(&self.paths.config) {
            self.status = format!("could not save what to show: {e:#}");
        }
        self.selected = keep.and_then(|id| self.visible().iter().position(|t| t.id == id)).unwrap_or(0);
        self.page_scroll = 0;
        self.action = 0;
    }

    fn show_key(&mut self, k: KeyEvent) {
        let n = self.show_rows().len();
        match k.code {
            KeyCode::Up => self.show_row = self.show_row.saturating_sub(1),
            KeyCode::Down if self.show_row + 1 >= n => self.focus = Focus::List,
            KeyCode::Down => self.show_row += 1,
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_show(self.show_row),
            // typing searches, as it does from the list
            KeyCode::Char(_) | KeyCode::Backspace => {
                self.focus = Focus::List;
                self.list_key(k);
            }
            _ => {}
        }
    }

    fn clamp(&mut self) {
        let n = self.visible().len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }

    fn select(&mut self, i: usize) {
        if i != self.selected {
            self.selected = i;
            self.page_scroll = 0;
            self.action = 0;
        }
    }

    /// Called every frame: drain worker messages, advance the preview.
    pub fn tick(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f64();
        self.last_tick = Instant::now();
        while let Ok(p) = self.rx.try_recv() {
            match p {
                Progress::Bytes { id, done, total } => {
                    let e = self.progress.entry(id).or_insert((0, 0, String::new()));
                    e.0 = done;
                    e.1 = total;
                }
                Progress::Status { id, msg } => {
                    log::info!("{id}: {msg}");
                    self.progress.entry(id).or_insert((0, 0, String::new())).2 = msg;
                }
                Progress::Done { id, installed } => {
                    self.progress.remove(&id);
                    self.status = format!("installed {id} {}", installed.version);
                    if !self.paths.bin_on_path()
                        && let Some(b) = &self.paths.bin
                    {
                        // the welcome screen explains this once; here a reminder is enough
                        self.status += &format!("; {} is not on your PATH, so start it from av", crate::paths::tilde(b));
                    }
                    let current = self.current().is_some_and(|t| t.id == id);
                    self.state.installed.insert(id, installed);
                    self.save();
                    if current {
                        // the obvious next step after installing is playing
                        if let Some(t) = self.current() {
                            self.action = self.actions(&t).iter().position(|(a, ..)| *a == Action::Play).unwrap_or(0);
                        }
                    }
                }
                Progress::Failed { id, error } => {
                    self.progress.remove(&id);
                    self.status = format!("{id}: install failed");
                    self.modal = Some(Modal::Message(format!("Installing {id} failed.\n\n{error}")));
                }
            }
        }
        while let Ok(CastMsg::Loaded(id, res)) = self.cast_rx.try_recv() {
            self.cast_loading.remove(&id);
            match res {
                Ok(cast) => {
                    log::debug!("cast {id}: {}x{}, {:.1}s", cast.cols, cast.rows, cast.duration);
                    let at = self.titles.iter().find(|t| t.id == id).and_then(|t| t.thumbnail_at).unwrap_or(0.0);
                    let mut p = Player::new(cast);
                    p.seek((at - 3.0).max(0.0));
                    if self.current().is_some_and(|t| t.id == id) {
                        self.player = Some((id, p));
                    }
                }
                Err(e) => {
                    log::warn!("cast {id}: {e}");
                    self.cast_failed.insert(id, e);
                }
            }
        }
        self.ensure_cast();
        if let Some((_, p)) = &mut self.player {
            p.tick(dt.min(0.25));
        }
    }

    /// Load the selected title's cast in the background (cached on disk).
    fn ensure_cast(&mut self) {
        let Some(t) = self.current() else { return };
        if self.player.as_ref().is_some_and(|(id, _)| *id == t.id) {
            return;
        }
        self.player = None;
        let Some(url) = &t.cast_url else { return };
        if self.cast_loading.contains(&t.id) || self.cast_failed.contains_key(&t.id) {
            return;
        }
        let url = catalog::resolve(&self.catalog_url, url);
        // keyed by URL too: a re-recorded cast gets a new URL from the catalog
        // build, and must not be shadowed by the old file in the cache
        let file = self.paths.cache.join("casts").join(format!("{}-{:016x}.cast", t.id, fnv1a(&url)));
        let (tx, id) = (self.cast_tx.clone(), t.id.clone());
        std::thread::spawn(move || {
            let res = (|| -> anyhow::Result<Cast> {
                let src = match std::fs::read_to_string(&file) {
                    Ok(s) => s,
                    Err(_) => {
                        let s = ureq::get(&url).call()?.body_mut().with_config().limit(256 << 20).read_to_string()?;
                        std::fs::create_dir_all(file.parent().unwrap())?;
                        std::fs::write(&file, &s)?;
                        s
                    }
                };
                Cast::parse(&src)
            })();
            let _ = tx.send(CastMsg::Loaded(id, res.map_err(|e| format!("{e:#}"))));
        });
        self.cast_loading.insert(t.id.clone());
    }

    /// None when there is nothing to show, else a placeholder message.
    pub fn cast_note(&self, t: &Title) -> Option<String> {
        if t.cast_url.is_none() {
            Some("no preview recorded yet".into())
        } else if let Some(e) = self.cast_failed.get(&t.id) {
            Some(format!("preview unavailable: {e}"))
        } else if self.cast_loading.contains(&t.id) {
            Some("loading preview…".into())
        } else {
            None
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.state.save() {
            self.status = format!("could not save state: {e:#}");
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        log::debug!("key {:?} {:?} focus={:?} modal={}", k.code, k.modifiers, self.focus, self.modal.is_some());
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if let Some(m) = self.modal.take() {
            self.on_modal_key(m, k);
            return;
        }
        match k.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Show | Focus::List => Focus::Actions,
                    Focus::Actions => Focus::Page,
                    Focus::Page => Focus::List,
                };
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Show | Focus::List => Focus::Page,
                    Focus::Actions => Focus::List,
                    Focus::Page => Focus::Actions,
                };
            }
            KeyCode::F(5) => self.refresh(true),
            KeyCode::Esc => {
                if self.focus != Focus::List {
                    self.focus = Focus::List;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.clamp();
                } else {
                    self.modal = Some(Modal::Quit);
                }
            }
            _ => match self.focus {
                Focus::Show => self.show_key(k),
                Focus::List => self.list_key(k),
                Focus::Actions => self.actions_key(k),
                Focus::Page => self.page_key(k),
            },
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        // later entries sit on top (buttons over the pane they are drawn in)
        let hit = self.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
        if !matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
            log::debug!("mouse {:?} at {},{} hit {hit:?}", m.kind, m.column, m.row);
        }
        if let Some(modal) = self.modal.take() {
            // a click closes a note; other dialogs stay keyboard-driven
            if !matches!(modal, Modal::Message(_)) || !matches!(m.kind, MouseEventKind::Down(_)) {
                self.modal = Some(modal);
            }
            return;
        }
        match (m.kind, hit) {
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::Row(i))) => {
                self.select(i);
                self.focus = Focus::List;
            }
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::ShowHead)) => {
                self.focus = if self.focus == Focus::Show { Focus::List } else { Focus::Show };
            }
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::ShowRow(i))) => {
                self.focus = Focus::Show;
                self.show_row = i;
                self.toggle_show(i);
            }
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::Button(i))) => {
                let Some(t) = self.current() else { return };
                self.focus = Focus::Actions;
                self.action = i;
                if let Some((a, _, true)) = self.actions(&t).get(i).cloned() {
                    self.run(a, &t);
                }
            }
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::Detail)) => self.focus = Focus::Page,
            // typing goes to the search box whenever the list has focus
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::Search | Hit::List)) => self.focus = Focus::List,
            (MouseEventKind::ScrollDown, Some(Hit::Row(_) | Hit::List)) => {
                let n = self.visible().len();
                self.select((self.selected + 1).min(n.saturating_sub(1)));
            }
            (MouseEventKind::ScrollUp, Some(Hit::Row(_) | Hit::List)) => self.select(self.selected.saturating_sub(1)),
            (MouseEventKind::ScrollDown, Some(Hit::Detail | Hit::Button(_))) => self.page_scroll += 3,
            (MouseEventKind::ScrollUp, Some(Hit::Detail | Hit::Button(_))) => {
                self.page_scroll = self.page_scroll.saturating_sub(3)
            }
            _ => {}
        }
    }

    fn list_key(&mut self, k: KeyEvent) {
        let n = self.visible().len();
        match k.code {
            // up from the top opens the show card above the list
            KeyCode::Up if self.selected == 0 => {
                self.focus = Focus::Show;
                self.show_row = 0;
            }
            KeyCode::Up => self.select(self.selected - 1),
            KeyCode::Down => self.select((self.selected + 1).min(n.saturating_sub(1))),
            KeyCode::PageUp => self.select(self.selected.saturating_sub(10)),
            KeyCode::PageDown => self.select((self.selected + 10).min(n.saturating_sub(1))),
            KeyCode::Home => self.select(0),
            KeyCode::End => self.select(n.saturating_sub(1)),
            KeyCode::Enter => {
                self.focus = Focus::Actions;
                self.action = 0;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = self.name_match();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.selected = self.name_match();
            }
            _ => {}
        }
    }

    fn actions_key(&mut self, k: KeyEvent) {
        let Some(t) = self.current() else { return };
        let acts = self.actions(&t);
        match k.code {
            KeyCode::Left => self.action = self.action.saturating_sub(1),
            KeyCode::Right => self.action = (self.action + 1).min(acts.len() - 1),
            KeyCode::Up | KeyCode::Down => self.focus = Focus::List,
            KeyCode::Enter => {
                if let Some((a, _, enabled)) = acts.get(self.action)
                    && *enabled
                {
                    self.run(*a, &t);
                }
            }
            _ => {}
        }
    }

    fn page_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up => self.page_scroll = self.page_scroll.saturating_sub(1),
            KeyCode::Down => self.page_scroll += 1,
            KeyCode::PageUp => self.page_scroll = self.page_scroll.saturating_sub(10),
            KeyCode::PageDown => self.page_scroll += 10,
            KeyCode::Char(' ') => {
                if let Some((_, p)) = &mut self.player {
                    p.playing = !p.playing;
                }
            }
            _ => {}
        }
    }

    fn run(&mut self, a: Action, t: &Title) {
        log::info!("action {a:?} on {}", t.id);
        match a {
            Action::Install => {
                install::spawn(
                    install::Job {
                        title: t.clone(),
                        catalog_url: self.catalog_url.clone(),
                        cache: self.paths.cache.clone(),
                        dest: self.paths.title_dir(&t.kind, &t.id),
                        bin: self.paths.bin.clone(),
                    },
                    self.tx.clone(),
                );
                self.progress.insert(t.id.clone(), (0, 0, "starting".into()));
            }
            Action::Uninstall => {
                self.uninstall_keep_data = true;
                self.modal = Some(Modal::Uninstall { id: t.id.clone() });
            }
            Action::Play => self.request_play(t, false),
            Action::Wishlist => {
                if !self.state.wishlist.remove(&t.id) {
                    self.state.wishlist.insert(t.id.clone());
                }
                self.save();
            }
        }
    }

    /// Checks before launching: terminal size against the title's minimum,
    /// then whether an update is waiting. Each asks; neither blocks.
    fn request_play(&mut self, t: &Title, size_ok: bool) {
        if !size_ok
            && let (Some(m), Ok((w, h))) = (t.min_terminal, ratatui::crossterm::terminal::size())
            && (w < m.cols || h < m.rows)
        {
            self.modal = Some(Modal::Choice {
                title: " terminal size ".into(),
                text: format!(
                    "{} needs a terminal of at least {}x{}. This one is {w}x{h}.\n\nEnlarge the window first, or play anyway.",
                    t.title, m.cols, m.rows
                ),
                options: vec![("Cancel".into(), Then::Cancel), ("Play anyway".into(), Then::Play { id: t.id.clone(), size_ok: true })],
                sel: 0,
            });
            return;
        }
        if !t.is_mud() && self.status_of(t) == Status::Update {
            let have = self.state.installed.get(&t.id).map(|i| i.version.clone()).unwrap_or_default();
            self.modal = Some(Modal::Choice {
                title: " update available ".into(),
                text: format!("{} {} is out; you have {have}.", t.title, t.version),
                options: vec![
                    ("Update first".into(), Then::Install(t.id.clone())),
                    (format!("Play {have}"), Then::Launch(t.id.clone())),
                ],
                sel: 0,
            });
            return;
        }
        self.launch = Some(t.clone());
    }

    fn choose(&mut self, then: Then) {
        let find = |app: &App, id: &str| app.titles.iter().find(|t| t.id == id).cloned();
        match then {
            Then::Cancel => {}
            Then::Play { id, size_ok } => {
                if let Some(t) = find(self, &id) {
                    self.request_play(&t, size_ok);
                }
            }
            Then::Launch(id) => self.launch = find(self, &id),
            Then::Install(id) => {
                if let Some(t) = find(self, &id) {
                    self.run(Action::Install, &t);
                }
            }
        }
    }

    /// Back from a title: record play time, then say how it went.
    pub fn lera(&self) -> Option<std::path::PathBuf> {
        crate::launch::lera(&self.paths)
    }

    pub fn finished(&mut self, t: &Title, o: crate::launch::Outcome) {
        let sessions = self.state.record_play(&t.id, o.seconds);
        self.save();
        let played = crate::launch::duration(o.seconds);
        match &o.status {
            Ok(st) => match crate::launch::failure(st) {
                None => {
                    self.status = format!("back from {}, played {played}", t.title);
                    // now and then, not every time: point at the developer's own donation page
                    if let Some(url) = &t.funding
                        && sessions.is_multiple_of(5)
                    {
                        self.status += &format!(". Enjoying it? Support its developer: {url}");
                    }
                }
                Some(how) => {
                    let mut text = format!("{} {how} after {played}.", t.title);
                    if let Some(log) = &o.log {
                        let tail = crate::launch::tail(log, 12);
                        if !tail.is_empty() {
                            text += &format!("\n\nLast lines it printed:\n\n{tail}");
                        }
                        text += &format!("\n\nFull log: {}", crate::paths::tilde(log));
                    }
                    self.status = format!("{} {how}", t.title);
                    self.modal = Some(Modal::Message(text));
                }
            },
            Err(e) if t.connect.is_some() && e.kind() == std::io::ErrorKind::NotFound => {
                self.status = "Neither Lera nor telnet is installed; install Lera to play MUDs".into();
            }
            Err(e) => {
                self.modal = Some(Modal::Message(format!("Could not start {}.\n\n{e}", t.title)));
            }
        }
        log::info!("{}: {}", t.id, self.status);
    }

    fn on_modal_key(&mut self, m: Modal, k: KeyEvent) {
        match m {
            Modal::Choice { title, text, options, mut sel } => match k.code {
                KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                    sel = sel.saturating_sub(1);
                    self.modal = Some(Modal::Choice { title, text, options, sel });
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                    sel = (sel + 1).min(options.len().saturating_sub(1));
                    self.modal = Some(Modal::Choice { title, text, options, sel });
                }
                KeyCode::Enter => {
                    if let Some((_, then)) = options.get(sel).cloned() {
                        self.choose(then);
                    }
                }
                KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Choice { title, text, options, sel }),
            },
            Modal::Quit => match k.code {
                KeyCode::Enter | KeyCode::Char('y') => self.quit = true,
                _ => {}
            },
            Modal::Message(_) => {}
            Modal::Uninstall { id } => match k.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                    self.uninstall_keep_data = !self.uninstall_keep_data;
                    self.modal = Some(Modal::Uninstall { id });
                }
                KeyCode::Enter => {
                    let Some(t) = self.titles.iter().find(|t| t.id == id).cloned() else { return };
                    let dest = self.paths.title_dir(&t.kind, &t.id);
                    match install::uninstall(&dest, self.paths.bin.as_deref(), &t.id) {
                        Ok(()) => {
                            self.state.installed.remove(&id);
                            self.save();
                            self.status = if self.uninstall_keep_data {
                                format!("uninstalled {id}; its saves and settings were kept")
                            } else {
                                // titles keep their data in their own config/data dirs;
                                // the manifest does not say where yet, so av cannot remove it
                                format!("uninstalled {id}; removing save data is not supported yet, nothing else was deleted")
                            };
                            self.action = 0;
                        }
                        Err(e) => self.modal = Some(Modal::Message(format!("Uninstall failed.\n\n{e:#}"))),
                    }
                }
                KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Uninstall { id }),
            },
        }
    }
}

/// FNV-1a, enough to tell cache keys apart without another dependency.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}
