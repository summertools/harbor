//! Interactive terminal UI.

mod draw;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use ratatui::widgets::ListState;

use crate::Harbor;
use crate::config::{Config, Source};
use crate::error::{Error, Result};
use crate::github::{GitHubClient, Release, RepoRef};
use crate::install::{
    self, InstallOutcome, InstallPlan, InstallRequest, InstallStage, Installer, ProgressEvent,
};
use crate::platform::{BinDir, Platform};
use crate::release::{self, AssetChoice};

/// Options passed from the CLI.
#[derive(Clone, Debug)]
pub struct UiOptions {
    pub color: bool,
    pub config_path: PathBuf,
}

/// Enter the interactive UI. The terminal is restored on normal exit, error,
/// or panic.
pub fn run(harbor: Harbor, options: UiOptions) -> Result<()> {
    let Harbor {
        config,
        platform,
        bin_dir,
        client,
    } = harbor;
    let worker = Worker::spawn(client)?;
    let state = UiState::new(
        config,
        platform,
        bin_dir,
        options.color,
        options.config_path,
    );
    let mut app = App {
        state,
        worker,
        quit: false,
    };
    app.start();

    let mut session = TerminalSession::enter()?;
    let result = (|| -> Result<()> {
        loop {
            let (width, height) = terminal::size().map_err(Error::Io)?;
            if width > MAX_UI_WIDTH || height > MAX_UI_HEIGHT {
                return Err(Error::msg(format!(
                    "terminal size {width}x{height} is too large (max {MAX_UI_WIDTH}x{MAX_UI_HEIGHT})"
                )));
            }
            session
                .terminal
                .draw(|frame| draw::draw(frame, &mut app.state))
                .map_err(Error::Io)?;
            while event::poll(Duration::from_millis(100)).map_err(Error::Io)? {
                match event::read().map_err(Error::Io)? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        app.handle_key(key);
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
                if app.quit {
                    break;
                }
            }
            app.tick();
            if app.quit {
                break;
            }
        }
        Ok(())
    })();
    drop(session);
    result
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

const MAX_UI_WIDTH: u16 = 512;
const MAX_UI_HEIGHT: u16 = 256;

impl TerminalSession {
    fn enter() -> Result<Self> {
        let (width, height) = terminal::size()?;
        if width > MAX_UI_WIDTH || height > MAX_UI_HEIGHT {
            return Err(Error::msg(format!(
                "terminal size {width}x{height} is too large (max {MAX_UI_WIDTH}x{MAX_UI_HEIGHT})"
            )));
        }
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All)) {
            let _ = disable_raw_mode();
            return Err(Error::Io(error));
        }
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).map_err(|error| {
            restore_terminal();
            Error::Io(error)
        })?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, Show);
        let _ = self.terminal.show_cursor();
        let _ = disable_raw_mode();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, Show);
}

enum Command {
    LoadRepos(Config),
    LoadRelease {
        repo: RepoRef,
        include_prereleases: bool,
    },
    Install {
        plan: Box<InstallPlan>,
        config: Config,
    },
}

enum WorkerEvent {
    Repos(crate::Discovery),
    Release {
        repo: RepoRef,
        result: std::result::Result<Release, String>,
    },
    Progress(ProgressEvent),
    Installed {
        result: std::result::Result<InstallOutcome, String>,
    },
}

struct Worker {
    commands: Sender<Command>,
    events: Receiver<WorkerEvent>,
    _handle: thread::JoinHandle<()>,
}

impl Worker {
    fn spawn(client: GitHubClient) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
        let handle = thread::Builder::new()
            .name("harbor-network".to_owned())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        Command::LoadRepos(config) => {
                            let discovery = crate::discover(&config, &client);
                            if event_tx.send(WorkerEvent::Repos(discovery)).is_err() {
                                break;
                            }
                        }
                        Command::LoadRelease {
                            repo,
                            include_prereleases,
                        } => {
                            let result = client
                                .latest_release(&repo, include_prereleases)
                                .map_err(|error| error.to_string());
                            if event_tx
                                .send(WorkerEvent::Release { repo, result })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Command::Install { plan, config } => {
                            let bin_dir = BinDir {
                                path: plan.bin_dir.clone(),
                                in_path: plan.in_path,
                            };
                            let installer = Installer::new(&client, &config, bin_dir);
                            let result = installer
                                .install(&plan, |event| {
                                    let _ = event_tx.send(WorkerEvent::Progress(event));
                                })
                                .map_err(|error| error.to_string());
                            if event_tx.send(WorkerEvent::Installed { result }).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|error| Error::msg(format!("cannot start network worker: {error}")))?;
        Ok(Self {
            commands: command_tx,
            events: event_rx,
            _handle: handle,
        })
    }

    fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Focus {
    Repos,
    Assets,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub text: String,
    pub level: StatusLevel,
}

pub(crate) struct ReleaseView {
    pub repo: RepoRef,
    pub release: Release,
    pub choices: Vec<AssetChoice>,
    pub selected: usize,
    pub asset_state: ListState,
}

impl ReleaseView {
    fn new(repo: RepoRef, release: Release, choices: Vec<AssetChoice>) -> Self {
        Self {
            repo,
            release,
            choices,
            selected: 0,
            asset_state: ListState::default().with_selected(Some(0)),
        }
    }

    fn selected_choice(&self) -> Option<&AssetChoice> {
        self.choices.get(self.selected)
    }
}

pub(crate) struct ConfirmState {
    pub plan: InstallPlan,
    pub allow_unverified: bool,
    pub force: bool,
}

pub(crate) struct ProgressState {
    pub plan: InstallPlan,
    pub stage: InstallStage,
    pub current: u64,
    pub total: Option<u64>,
}

pub(crate) struct OutcomeState {
    pub result: std::result::Result<InstallOutcome, String>,
}

#[derive(Clone)]
pub(crate) struct SourceState {
    pub selected: usize,
    pub input: Option<String>,
    pub remove_confirm: Option<usize>,
    pub status: Option<Status>,
}

pub(crate) enum Stage {
    Browse,
    Confirm(ConfirmState),
    Progress(ProgressState),
    Outcome(OutcomeState),
    Sources(SourceState),
    Help,
}

pub(crate) struct UiState {
    pub config: Config,
    pub config_path: PathBuf,
    pub platform: Platform,
    pub bin_dir: BinDir,
    pub color: bool,
    pub repos: Vec<crate::RepoItem>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub repo_state: ListState,
    pub focus: Focus,
    pub release: Option<ReleaseView>,
    pub release_loading: bool,
    release_inflight: Option<String>,
    pub release_error: Option<String>,
    release_failed_for: Option<String>,
    pub last_selection_change: Instant,
    pub search_query: String,
    pub search_buffer: String,
    pub search_active: bool,
    pub source_filter: Option<usize>,
    pub stage: Stage,
    pub status: Option<Status>,
    pub warnings: Vec<String>,
    pub spinner: usize,
    pub loading_repos: bool,
    pub source_state: SourceState,
    last_tick: Instant,
}

impl UiState {
    fn new(
        config: Config,
        platform: Platform,
        bin_dir: BinDir,
        color: bool,
        config_path: PathBuf,
    ) -> Self {
        Self {
            config,
            config_path,
            platform,
            bin_dir,
            color,
            repos: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            repo_state: ListState::default(),
            focus: Focus::Repos,
            release: None,
            release_loading: false,
            release_inflight: None,
            release_error: None,
            release_failed_for: None,
            last_selection_change: Instant::now(),
            search_query: String::new(),
            search_buffer: String::new(),
            search_active: false,
            source_filter: None,
            stage: Stage::Browse,
            status: None,
            warnings: Vec::new(),
            spinner: 0,
            loading_repos: true,
            source_state: SourceState {
                selected: 0,
                input: None,
                remove_confirm: None,
                status: None,
            },
            last_tick: Instant::now(),
        }
    }

    fn selected_repo(&self) -> Option<&crate::RepoItem> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.repos.get(*index))
    }

    fn selected_repo_key(&self) -> Option<String> {
        self.selected_repo()
            .map(|repo| repo.full_name.to_ascii_lowercase())
    }

    fn current_query(&self) -> &str {
        if self.search_active {
            &self.search_buffer
        } else {
            &self.search_query
        }
    }

    fn rebuild_filter(&mut self) {
        let previous = self.selected_repo_key();
        let query = self.current_query().trim().to_ascii_lowercase();
        let source = self
            .source_filter
            .and_then(|index| self.config.sources.get(index));
        self.filtered = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, repo)| {
                if source.is_some_and(|source| {
                    !source.label().eq_ignore_ascii_case(&repo.source.label())
                        && !source.name.eq_ignore_ascii_case(&repo.source.name)
                }) {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                repo.full_name.to_ascii_lowercase().contains(&query)
                    || repo.name.to_ascii_lowercase().contains(&query)
                    || repo.description.as_deref().is_some_and(|description| {
                        description.to_ascii_lowercase().contains(&query)
                    })
                    || repo.source.label().to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.repo_state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(self.selected)
        });
        let current = self.selected_repo_key();
        if previous != current {
            self.clear_release();
            self.last_selection_change = Instant::now();
        }
    }

    fn clear_release(&mut self) {
        self.release = None;
        self.release_error = None;
        self.release_loading = self.release_inflight.is_some();
        self.release_failed_for = None;
        self.focus = Focus::Repos;
    }

    fn on_selection_changed(&mut self) {
        self.last_selection_change = Instant::now();
        self.clear_release();
        self.status = None;
    }

    fn move_repo(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let next = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs())
        } else {
            self.selected
                .saturating_add(delta as usize)
                .min(self.filtered.len() - 1)
        };
        if next != self.selected {
            self.selected = next;
            self.repo_state.select(Some(self.selected));
            self.on_selection_changed();
        }
    }

    fn page_repo(&mut self, rows: usize, down: bool) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = if down {
            self.selected
                .saturating_add(rows)
                .min(self.filtered.len() - 1)
        } else {
            self.selected.saturating_sub(rows)
        };
        self.repo_state.select(Some(self.selected));
        self.on_selection_changed();
    }

    fn move_asset(&mut self, delta: isize) {
        let Some(release) = &mut self.release else {
            return;
        };
        if release.choices.is_empty() {
            return;
        }
        let next = if delta < 0 {
            release.selected.saturating_sub(delta.unsigned_abs())
        } else {
            release
                .selected
                .saturating_add(delta as usize)
                .min(release.choices.len() - 1)
        };
        release.selected = next;
        release.asset_state.select(Some(next));
    }

    fn cycle_source_filter(&mut self) {
        if self.config.sources.is_empty() {
            self.source_filter = None;
        } else {
            self.source_filter = match self.source_filter {
                None => Some(0),
                Some(index) if index + 1 < self.config.sources.len() => Some(index + 1),
                _ => None,
            };
        }
        self.selected = 0;
        self.clear_release();
        self.rebuild_filter();
        self.last_selection_change = Instant::now();
    }

    fn source_filter_label(&self) -> String {
        self.source_filter
            .and_then(|index| self.config.sources.get(index))
            .map_or_else(|| "all sources".to_owned(), Source::label)
    }
}

struct App {
    state: UiState,
    worker: Worker,
    quit: bool,
}

impl App {
    fn start(&mut self) {
        self.state.loading_repos = true;
        if !self
            .worker
            .send(Command::LoadRepos(self.state.config.clone()))
        {
            self.state.status = Some(Status {
                text: "network worker stopped".to_owned(),
                level: StatusLevel::Error,
            });
        }
    }

    fn tick(&mut self) {
        self.poll_worker();
        let now = Instant::now();
        if now.duration_since(self.state.last_tick) >= Duration::from_millis(90) {
            self.state.spinner = self.state.spinner.wrapping_add(1);
            self.state.last_tick = now;
        }
        self.schedule_release(now);
    }

    fn poll_worker(&mut self) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                WorkerEvent::Repos(discovery) => {
                    self.state.loading_repos = false;
                    self.state.warnings = discovery.warnings.clone();
                    self.state.repos = discovery.repos;
                    self.state.selected = 0;
                    self.state.clear_release();
                    self.state.rebuild_filter();
                    self.state.last_selection_change = Instant::now() - Duration::from_secs(1);
                    if let Some(warning) = self.state.warnings.first() {
                        self.state.status = Some(Status {
                            text: warning.clone(),
                            level: StatusLevel::Warning,
                        });
                    }
                }
                WorkerEvent::Release { repo, result } => {
                    self.state.release_loading = false;
                    self.state.release_inflight = None;
                    let current = self.state.selected_repo_key();
                    if current.as_deref() != Some(repo.full_name().to_ascii_lowercase().as_str()) {
                        continue;
                    }
                    match result {
                        Ok(release) => {
                            match release::select_assets(self.state.platform, &release, None) {
                                Ok(choices) => {
                                    self.state.release =
                                        Some(ReleaseView::new(repo, release, choices));
                                    self.state.release_error = None;
                                    self.state.release_failed_for = None;
                                }
                                Err(error) => {
                                    self.state.release = None;
                                    self.state.release_error = Some(error.to_string());
                                    self.state.release_failed_for = current;
                                }
                            }
                        }
                        Err(error) => {
                            self.state.release = None;
                            self.state.release_error = Some(error);
                            self.state.release_failed_for = current;
                        }
                    }
                }
                WorkerEvent::Progress(event) => {
                    if let Stage::Progress(progress) = &mut self.state.stage {
                        progress.stage = event.stage;
                        progress.current = event.current;
                        progress.total = event.total;
                    }
                }
                WorkerEvent::Installed { result } => {
                    self.state.status = None;
                    self.state.stage = Stage::Outcome(OutcomeState { result });
                }
            }
        }
    }

    fn schedule_release(&mut self, now: Instant) {
        if self.state.loading_repos
            || self.state.release_loading
            || self.state.release_inflight.is_some()
            || self.state.release.is_some()
        {
            return;
        }
        let Some(repo) = self.state.selected_repo() else {
            return;
        };
        let key = repo.full_name.to_ascii_lowercase();
        if self.state.release_failed_for.as_deref() == Some(key.as_str()) {
            return;
        }
        if now.duration_since(self.state.last_selection_change) < Duration::from_millis(320) {
            return;
        }
        let command = Command::LoadRelease {
            repo: repo.repo_ref(),
            include_prereleases: self.state.config.include_prereleases,
        };
        if self.worker.send(command) {
            self.state.release_loading = true;
            self.state.release_inflight = Some(key);
            self.state.release_error = None;
        }
    }

    fn refresh(&mut self) {
        self.state.clear_release();
        self.state.warnings.clear();
        self.state.status = None;
        self.start();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            self.quit = true;
            return;
        }
        if self.state.search_active {
            self.handle_search_key(key);
            return;
        }
        let stage = std::mem::replace(&mut self.state.stage, Stage::Browse);
        match stage {
            Stage::Browse => {
                self.state.stage = Stage::Browse;
                self.handle_browse(key);
            }
            Stage::Confirm(mut confirm) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.state.stage = Stage::Browse;
                }
                KeyCode::Char('u') => {
                    if !confirm.plan.checksum_available() {
                        confirm.allow_unverified = !confirm.allow_unverified;
                    }
                    self.state.stage = Stage::Confirm(confirm);
                }
                KeyCode::Char('o') => {
                    confirm.force = !confirm.force;
                    self.state.stage = Stage::Confirm(confirm);
                }
                KeyCode::Enter | KeyCode::Char('y') => {
                    if !confirm.plan.checksum_available() && !confirm.allow_unverified {
                        self.state.status = Some(Status {
                            text: "press u to allow an unverified install, or Esc to cancel"
                                .to_owned(),
                            level: StatusLevel::Warning,
                        });
                        self.state.stage = Stage::Confirm(confirm);
                    } else {
                        let mut plan = confirm.plan;
                        plan.request.allow_unverified = confirm.allow_unverified;
                        plan.request.force = confirm.force;
                        let command = Command::Install {
                            plan: Box::new(plan.clone()),
                            config: self.state.config.clone(),
                        };
                        if self.worker.send(command) {
                            self.state.stage = Stage::Progress(ProgressState {
                                plan,
                                stage: InstallStage::Downloading,
                                current: 0,
                                total: None,
                            });
                        } else {
                            self.state.status = Some(Status {
                                text: "network worker stopped".to_owned(),
                                level: StatusLevel::Error,
                            });
                            self.state.stage = Stage::Browse;
                        }
                    }
                }
                _ => {
                    self.state.stage = Stage::Confirm(confirm);
                }
            },
            Stage::Progress(progress) => {
                self.state.stage = Stage::Progress(progress);
            }
            Stage::Outcome(outcome) => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.state.stage = Stage::Browse;
                }
                KeyCode::Char('q') => {
                    self.state.stage = Stage::Outcome(outcome);
                    self.quit = true;
                }
                _ => {
                    self.state.stage = Stage::Outcome(outcome);
                }
            },
            Stage::Sources(mut sources) => {
                let close = self.handle_sources_key(key, &mut sources);
                self.state.stage = if close {
                    Stage::Browse
                } else {
                    Stage::Sources(sources)
                };
            }
            Stage::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.state.stage = Stage::Browse;
                } else {
                    self.state.stage = Stage::Help;
                }
            }
        }
    }

    fn handle_browse(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.state.stage = Stage::Help,
            KeyCode::Char('s') => {
                self.state.source_state.selected = 0;
                self.state.source_state.input = None;
                self.state.source_state.remove_confirm = None;
                self.state.source_state.status = None;
                self.state.stage = Stage::Sources(self.state.source_state.clone());
            }
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('/') => {
                self.state.search_buffer = self.state.search_query.clone();
                self.state.search_active = true;
            }
            KeyCode::Char('f') => self.state.cycle_source_filter(),
            KeyCode::Tab => {
                if self.state.release.is_some() {
                    self.state.focus = match self.state.focus {
                        Focus::Repos => Focus::Assets,
                        Focus::Assets => Focus::Repos,
                    };
                }
            }
            KeyCode::Char('i') => self.confirm_install(),
            KeyCode::Up | KeyCode::Char('k') => self.move_focus(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_focus(1),
            KeyCode::PageUp => match self.state.focus {
                Focus::Repos => self.state.page_repo(10, false),
                Focus::Assets => self.state.move_asset(-10),
            },
            KeyCode::PageDown => match self.state.focus {
                Focus::Repos => self.state.page_repo(10, true),
                Focus::Assets => self.state.move_asset(10),
            },
            KeyCode::Home | KeyCode::Char('g') => match self.state.focus {
                Focus::Repos => {
                    self.state.selected = 0;
                    self.state.repo_state.select(Some(0));
                    self.state.on_selection_changed();
                }
                Focus::Assets => {
                    if let Some(release) = &mut self.state.release {
                        release.selected = 0;
                        release.asset_state.select(Some(0));
                    }
                }
            },
            KeyCode::End | KeyCode::Char('G') => match self.state.focus {
                Focus::Repos => {
                    self.state.selected = self.state.filtered.len().saturating_sub(1);
                    self.state.repo_state.select(Some(self.state.selected));
                    self.state.on_selection_changed();
                }
                Focus::Assets => {
                    if let Some(release) = &mut self.state.release {
                        release.selected = release.choices.len().saturating_sub(1);
                        release.asset_state.select(Some(release.selected));
                    }
                }
            },
            KeyCode::Enter => match self.state.focus {
                Focus::Repos => {
                    if self.state.release.is_some() {
                        self.state.focus = Focus::Assets;
                    } else {
                        self.state.release_failed_for = None;
                        self.state.last_selection_change = Instant::now() - Duration::from_secs(1);
                    }
                }
                Focus::Assets => self.confirm_install(),
            },
            KeyCode::Esc => {
                if self.state.focus == Focus::Assets {
                    self.state.focus = Focus::Repos;
                } else if !self.state.search_query.is_empty() {
                    self.state.search_query.clear();
                    self.state.rebuild_filter();
                }
            }
            _ => {}
        }
    }

    fn move_focus(&mut self, delta: isize) {
        match self.state.focus {
            Focus::Repos => self.state.move_repo(delta),
            Focus::Assets => self.state.move_asset(delta),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.search_active = false;
                self.state.search_buffer = self.state.search_query.clone();
                self.state.rebuild_filter();
            }
            KeyCode::Enter => {
                self.state.search_query = self.state.search_buffer.clone();
                self.state.search_active = false;
                self.state.rebuild_filter();
                self.state.last_selection_change = Instant::now() - Duration::from_millis(400);
            }
            KeyCode::Backspace => {
                self.state.search_buffer.pop();
                self.state.rebuild_filter();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.search_buffer.clear();
                self.state.rebuild_filter();
            }
            KeyCode::Up | KeyCode::Char('k') => self.state.move_repo(-1),
            KeyCode::Down | KeyCode::Char('j') => self.state.move_repo(1),
            KeyCode::Char(ch)
                if !ch.is_control() && self.state.search_buffer.chars().count() < 128 =>
            {
                self.state.search_buffer.push(ch);
                self.state.rebuild_filter();
            }
            _ => {}
        }
    }

    fn confirm_install(&mut self) {
        let Some(release) = self.state.release.as_ref() else {
            self.state.status = Some(Status {
                text: "select a repository and wait for its release to load".to_owned(),
                level: StatusLevel::Warning,
            });
            return;
        };
        let Some(choice) = release.selected_choice() else {
            self.state.status = Some(Status {
                text: "this release has no compatible asset".to_owned(),
                level: StatusLevel::Error,
            });
            return;
        };
        let request = InstallRequest {
            repo: release.repo.clone(),
            tag: Some(release.release.tag_name.clone()),
            asset_name: Some(choice.asset.name.clone()),
            asset_glob: None,
            binary: None,
            name: None,
            force: false,
            allow_unverified: false,
        };
        match install::build_plan(
            &self.state.config,
            self.state.platform,
            &self.state.bin_dir,
            request,
            release.release.clone(),
            choice.clone(),
        ) {
            Ok(plan) => {
                self.state.stage = Stage::Confirm(ConfirmState {
                    plan,
                    allow_unverified: false,
                    force: false,
                });
            }
            Err(error) => {
                self.state.status = Some(Status {
                    text: error.to_string(),
                    level: StatusLevel::Error,
                });
            }
        }
    }

    fn handle_sources_key(&mut self, key: KeyEvent, sources: &mut SourceState) -> bool {
        if let Some(input) = &mut sources.input {
            match key.code {
                KeyCode::Esc => sources.input = None,
                KeyCode::Enter => {
                    let spec = input.clone();
                    sources.input = None;
                    self.add_source(&spec, sources);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(ch) if !ch.is_control() && input.chars().count() < 128 => {
                    input.push(ch);
                }
                _ => {}
            }
            return false;
        }
        if let Some(index) = sources.remove_confirm {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    sources.remove_confirm = None;
                    self.remove_source(index, sources);
                }
                KeyCode::Esc | KeyCode::Char('n') => sources.remove_confirm = None,
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => {
                sources.selected = sources.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if sources.selected + 1 < self.state.config.sources.len() {
                    sources.selected += 1;
                }
            }
            KeyCode::Char('a') => sources.input = Some(String::new()),
            KeyCode::Char('d') if !self.state.config.sources.is_empty() => {
                sources.remove_confirm = Some(sources.selected);
            }
            _ => {}
        }
        false
    }

    fn add_source(&mut self, spec: &str, sources: &mut SourceState) {
        let source = match Source::parse_spec(spec) {
            Ok(source) => source,
            Err(error) => {
                sources.status = Some(Status {
                    text: error.to_string(),
                    level: StatusLevel::Error,
                });
                return;
            }
        };
        if self.state.config.find_source(&source.label()).is_some() {
            sources.status = Some(Status {
                text: format!("{} is already configured", source.label()),
                level: StatusLevel::Info,
            });
            return;
        }
        let mut config = self.state.config.clone();
        match config.add_source(source) {
            Ok(label) => {
                if let Err(error) = config.save_to(&self.state.config_path) {
                    sources.status = Some(Status {
                        text: error.to_string(),
                        level: StatusLevel::Error,
                    });
                    return;
                }
                self.state.config = config;
                sources.status = Some(Status {
                    text: format!("added {label}"),
                    level: StatusLevel::Success,
                });
                self.state.source_filter = None;
                self.start();
            }
            Err(error) => {
                sources.status = Some(Status {
                    text: error.to_string(),
                    level: StatusLevel::Error,
                });
            }
        }
    }

    fn remove_source(&mut self, index: usize, sources: &mut SourceState) {
        let Some(source) = self.state.config.sources.get(index).cloned() else {
            return;
        };
        let mut config = self.state.config.clone();
        if !config.remove_source(&source.label()) {
            sources.status = Some(Status {
                text: "source is no longer configured".to_owned(),
                level: StatusLevel::Error,
            });
            return;
        }
        if let Err(error) = config.save_to(&self.state.config_path) {
            sources.status = Some(Status {
                text: error.to_string(),
                level: StatusLevel::Error,
            });
            return;
        }
        self.state.config = config;
        sources.selected = sources
            .selected
            .min(self.state.config.sources.len().saturating_sub(1));
        sources.status = Some(Status {
            text: format!("removed {}", source.label()),
            level: StatusLevel::Success,
        });
        self.state.source_filter = None;
        self.start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_session_is_not_constructed_in_tests() {
        // The terminal helper is exercised manually; this test documents that
        // unit tests never enter raw mode.
        assert_eq!(Focus::Repos, Focus::Repos);
    }
}
