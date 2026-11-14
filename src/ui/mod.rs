//! Interactive terminal UI.

mod draw;

use std::collections::{HashMap, HashSet, VecDeque};
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
use crate::format;
use crate::github::{GitHubClient, Release, RepoRef};
use crate::install::{
    self, InstallOutcome, InstallPlan, InstallRequest, InstallStage, InstalledPackage, Installer,
    Ledger, ProgressEvent, installed_packages,
};
use crate::platform::{BinDir, Platform};
use crate::release::{self, AssetChoice};
use crate::update::AutoUpdatePlan;

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
        auto: bool,
    },
}

enum WorkerEvent {
    Repos(crate::Discovery),
    Release {
        repo: RepoRef,
        result: std::result::Result<Release, String>,
        no_release: bool,
    },
    Progress {
        event: ProgressEvent,
        auto: bool,
    },
    Installed {
        result: std::result::Result<InstallOutcome, String>,
        auto: bool,
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
                            let (result, no_release) =
                                match client.latest_release(&repo, include_prereleases) {
                                    Ok(release) => (Ok(release), false),
                                    Err(Error::NotFound(message) | Error::NoRelease(message)) => {
                                        (Err(message), true)
                                    }
                                    Err(error) => (Err(error.to_string()), false),
                                };
                            if event_tx
                                .send(WorkerEvent::Release {
                                    repo,
                                    result,
                                    no_release,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Command::Install { plan, config, auto } => {
                            let bin_dir = BinDir {
                                path: plan.bin_dir.clone(),
                                in_path: plan.in_path,
                            };
                            let installer = Installer::new(&client, &config, bin_dir);
                            let result = installer
                                .install(&plan, |event| {
                                    let _ = event_tx.send(WorkerEvent::Progress { event, auto });
                                })
                                .map_err(|error| error.to_string());
                            if event_tx
                                .send(WorkerEvent::Installed { result, auto })
                                .is_err()
                            {
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

/// Status filter shown above the repository list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusFilter {
    All,
    Installed,
    Updates,
    NotInstalled,
}

impl StatusFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Installed,
            Self::Installed => Self::Updates,
            Self::Updates => Self::NotInstalled,
            Self::NotInstalled => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Installed => "installed",
            Self::Updates => "updates",
            Self::NotInstalled => "not installed",
        }
    }
}

/// Why a latest release could not be shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseErrorKind {
    NoRelease,
    Unavailable,
}

pub(crate) struct AutoUpdateActivity {
    pub plan: AutoUpdatePlan,
    pub stage: InstallStage,
    pub current: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct ReleaseError {
    pub message: String,
    pub kind: ReleaseErrorKind,
}

/// Latest-release metadata cached for status chips and quick browsing.
#[derive(Clone, Debug)]
pub(crate) struct LatestInfo {
    pub release: Release,
    pub compatible: usize,
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
    pub others: Vec<AssetChoice>,
    pub selected: usize,
    pub asset_state: ListState,
}

impl ReleaseView {
    fn new(repo: RepoRef, release: Release, catalog: release::AssetCatalog) -> Self {
        let selected = usize::from(!catalog.compatible.is_empty());
        Self {
            repo,
            release,
            choices: catalog.compatible,
            others: catalog.incompatible,
            selected: 0,
            asset_state: ListState::default()
                .with_selected(Some(selected).filter(|value| *value > 0)),
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
    pub installed_version: Option<String>,
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
    pub release_error: Option<ReleaseError>,
    pub ledger: Ledger,
    pub ledger_error: Option<String>,
    pub installed: Vec<InstalledPackage>,
    installed_by_repo: HashMap<String, InstalledPackage>,
    pub latest: HashMap<String, LatestInfo>,
    latest_errors: HashMap<String, ReleaseError>,
    status_inflight: HashSet<String>,
    pub status_filter: StatusFilter,
    auto_queue: VecDeque<AutoUpdatePlan>,
    auto_active: Option<AutoUpdateActivity>,
    auto_attempted: HashSet<String>,
    auto_scan_done: bool,
    auto_updated: usize,
    auto_failed: usize,
    auto_skipped: usize,
    quit_after_auto: bool,
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
        let mut state = Self {
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
            ledger: Ledger::default(),
            ledger_error: None,
            installed: Vec::new(),
            installed_by_repo: HashMap::new(),
            latest: HashMap::new(),
            latest_errors: HashMap::new(),
            status_inflight: HashSet::new(),
            status_filter: StatusFilter::All,
            auto_queue: VecDeque::new(),
            auto_active: None,
            auto_attempted: HashSet::new(),
            auto_scan_done: false,
            auto_updated: 0,
            auto_failed: 0,
            auto_skipped: 0,
            quit_after_auto: false,
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
        };
        match Ledger::load() {
            Ok(ledger) => {
                state.ledger = ledger;
                state.refresh_installed();
            }
            Err(error) => state.ledger_error = Some(error.to_string()),
        }
        state
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
                if !self.status_matches(repo) {
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
                    || repo.display_source().to_ascii_lowercase().contains(&query)
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

    fn auto_activity_line(&self) -> Option<String> {
        let activity = self.auto_active.as_ref()?;
        let progress = match activity.total.filter(|total| *total > 0) {
            Some(total) => format!(
                "  {:.0}%  {}/{}",
                (activity.current as f64 / total as f64).clamp(0.0, 1.0) * 100.0,
                format::bytes(activity.current),
                format::bytes(total)
            ),
            None => String::new(),
        };
        Some(format!(
            "auto-update {} {} → {} · {}{}{}",
            activity.plan.package.repo,
            activity.plan.from_version(),
            activity.plan.to_version(),
            activity.stage.label(),
            progress,
            if self.quit_after_auto {
                " · quitting after this update"
            } else {
                ""
            }
        ))
    }

    fn refresh_installed(&mut self) {
        self.installed = installed_packages(&self.ledger);
        self.installed_by_repo = self
            .installed
            .iter()
            .cloned()
            .map(|package| (package.repo.to_ascii_lowercase(), package))
            .collect();
    }

    fn reload_ledger(&mut self) {
        match Ledger::load() {
            Ok(ledger) => {
                self.ledger = ledger;
                self.ledger_error = None;
                self.refresh_installed();
                self.merge_ledger_repos();
            }
            Err(error) => self.ledger_error = Some(error.to_string()),
        }
    }

    /// Add synthetic repository rows for managed binaries whose source was
    /// removed from the config. They remain visible and updateable.
    fn merge_ledger_repos(&mut self) {
        crate::merge_installed_repos(&mut self.repos, &self.installed);
    }

    fn installed_for(&self, repo: &crate::RepoItem) -> Option<&InstalledPackage> {
        self.installed_by_repo
            .get(&repo.full_name.to_ascii_lowercase())
    }

    fn latest_for(&self, repo: &crate::RepoItem) -> Option<&LatestInfo> {
        self.latest.get(&repo.full_name.to_ascii_lowercase())
    }

    fn has_update(&self, repo: &crate::RepoItem) -> bool {
        let Some(package) = self.installed_for(repo) else {
            return false;
        };
        self.latest_for(repo).is_some_and(|latest| {
            latest.compatible > 0 && latest.release.tag_name != package.version
        })
    }

    fn status_matches(&self, repo: &crate::RepoItem) -> bool {
        match self.status_filter {
            StatusFilter::All => true,
            StatusFilter::Installed => self.installed_for(repo).is_some(),
            StatusFilter::Updates => self.has_update(repo),
            StatusFilter::NotInstalled => self.installed_for(repo).is_none(),
        }
    }

    fn status_counts(&self) -> (usize, usize, usize) {
        let mut installed = 0usize;
        let mut updates = 0usize;
        let mut available = 0usize;
        for repo in &self.repos {
            if self.installed_for(repo).is_some() {
                installed += 1;
                if self.has_update(repo) {
                    updates += 1;
                }
            } else {
                available += 1;
            }
        }
        (installed, updates, available)
    }

    fn cycle_status_filter(&mut self) {
        self.status_filter = self.status_filter.next();
        self.selected = 0;
        self.clear_release();
        self.rebuild_filter();
        self.last_selection_change = Instant::now();
    }

    fn cache_release(&mut self, repo: &RepoRef, release: Release) {
        let catalog = release::catalog_assets(self.platform, &release);
        self.latest.insert(
            repo.full_name().to_ascii_lowercase(),
            LatestInfo {
                release,
                compatible: catalog.compatible.len(),
            },
        );
    }

    fn release_view_for(&mut self, repo: RepoRef, release: Release) -> ReleaseView {
        let catalog = release::catalog_assets(self.platform, &release);
        ReleaseView::new(repo, release, catalog)
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
        self.schedule_status_prefetch();
        self.schedule_auto_updates();
    }

    fn poll_worker(&mut self) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                WorkerEvent::Repos(discovery) => {
                    self.state.loading_repos = false;
                    self.state.warnings = discovery.warnings.clone();
                    self.state.repos = discovery.repos;
                    self.state.merge_ledger_repos();
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
                WorkerEvent::Release {
                    repo,
                    result,
                    no_release,
                } => {
                    let key = repo.full_name().to_ascii_lowercase();
                    self.state.status_inflight.remove(&key);
                    let selected = self.state.selected_repo_key();
                    let selected = selected.as_deref() == Some(key.as_str());
                    if self.state.release_inflight.as_deref() == Some(key.as_str()) {
                        self.state.release_loading = false;
                        self.state.release_inflight = None;
                    }
                    match result {
                        Ok(release) => {
                            self.state.cache_release(&repo, release.clone());
                            self.state.latest_errors.remove(&key);
                            if selected {
                                let view = self.state.release_view_for(repo, release);
                                self.state.release = Some(view);
                                self.state.release_error = None;
                            }
                        }
                        Err(message) => {
                            let error = ReleaseError {
                                message,
                                kind: if no_release {
                                    ReleaseErrorKind::NoRelease
                                } else {
                                    ReleaseErrorKind::Unavailable
                                },
                            };
                            self.state.latest_errors.insert(key.clone(), error.clone());
                            if selected {
                                self.state.release = None;
                                self.state.release_error = Some(error);
                            }
                        }
                    }
                    self.state.rebuild_filter();
                }
                WorkerEvent::Progress { event, auto } => {
                    if auto {
                        if let Some(activity) = &mut self.state.auto_active {
                            activity.stage = event.stage;
                            activity.current = event.current;
                            activity.total = event.total;
                        }
                    } else if let Stage::Progress(progress) = &mut self.state.stage {
                        progress.stage = event.stage;
                        progress.current = event.current;
                        progress.total = event.total;
                    }
                }
                WorkerEvent::Installed { result, auto } => {
                    if result.is_ok() {
                        self.state.reload_ledger();
                        self.state.merge_ledger_repos();
                        self.state.rebuild_filter();
                    }
                    if auto {
                        self.finish_auto_update(result);
                    } else {
                        self.state.status = None;
                        self.state.stage = Stage::Outcome(OutcomeState { result });
                    }
                }
            }
        }
    }

    fn schedule_release(&mut self, now: Instant) {
        if self.state.loading_repos || self.state.release_loading || self.state.release.is_some() {
            return;
        }
        let Some(repo) = self.state.selected_repo() else {
            return;
        };
        let repo_ref = repo.repo_ref();
        let key = repo.full_name.to_ascii_lowercase();

        if let Some(info) = self.state.latest.get(&key) {
            let view = self.state.release_view_for(repo_ref, info.release.clone());
            self.state.release = Some(view);
            self.state.release_error = None;
            return;
        }
        if let Some(error) = self.state.latest_errors.get(&key).cloned() {
            self.state.release_error = Some(error);
            return;
        }
        if !self.state.status_inflight.insert(key.clone()) {
            // A prefetch for this repository is already running. Adopt it as
            // the selected request so the spinner and result are applied.
            self.state.release_loading = true;
            self.state.release_inflight = Some(key);
            return;
        }
        if now.duration_since(self.state.last_selection_change) < Duration::from_millis(320) {
            self.state.status_inflight.remove(&key);
            return;
        }
        let command = Command::LoadRelease {
            repo: repo_ref,
            include_prereleases: self.state.config.include_prereleases,
        };
        if self.worker.send(command) {
            self.state.release_loading = true;
            self.state.release_inflight = Some(key);
            self.state.release_error = None;
        } else {
            self.state.status_inflight.remove(&key);
        }
    }

    /// Fetch latest release metadata for installed repositories so update
    /// badges and the "updates" filter are meaningful without opening each one.
    fn schedule_status_prefetch(&mut self) {
        if self.state.loading_repos || self.state.installed.is_empty() {
            return;
        }
        let selected = self.state.selected_repo_key();
        let mut sent = 0usize;
        let mut keys: Vec<String> = self
            .state
            .installed
            .iter()
            .map(|package| package.repo.to_ascii_lowercase())
            .collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            if sent >= 4 {
                break;
            }
            if selected.as_deref() == Some(key.as_str())
                || self.state.latest.contains_key(&key)
                || self.state.latest_errors.contains_key(&key)
                || self.state.status_inflight.contains(&key)
            {
                continue;
            }
            let Some(repo) = self
                .state
                .repos
                .iter()
                .find(|repo| repo.full_name.eq_ignore_ascii_case(&key))
            else {
                continue;
            };
            let command = Command::LoadRelease {
                repo: repo.repo_ref(),
                include_prereleases: self.state.config.include_prereleases,
            };
            if self.worker.send(command) {
                self.state.status_inflight.insert(key);
                sent += 1;
            }
        }
    }

    fn schedule_auto_updates(&mut self) {
        if !self.state.config.auto_update {
            return;
        }
        if self.state.auto_active.is_some() {
            return;
        }
        if !self.state.auto_queue.is_empty() {
            self.start_next_auto_update();
            return;
        }
        if self.state.auto_scan_done {
            return;
        }
        if !matches!(self.state.stage, Stage::Browse)
            || self.state.loading_repos
            || self.state.release_loading
            || !self.state.status_inflight.is_empty()
        {
            return;
        }
        self.prepare_auto_updates();
    }

    fn prepare_auto_updates(&mut self) {
        // Wait until every managed repository has either latest-release
        // metadata or a recorded fetch error. Marking the scan done while a
        // request is still in flight would silently skip that package until
        // the next manual refresh.
        let ready = self.state.installed.iter().all(|package| {
            let key = package.repo.to_ascii_lowercase();
            self.state.latest.contains_key(&key) || self.state.latest_errors.contains_key(&key)
        });
        if !ready {
            return;
        }
        self.state.auto_scan_done = true;
        let checksum_config = crate::update::checksum_only_config(self.state.config.clone());
        let platform = self.state.platform;
        let packages = install::ledger_packages(&self.state.ledger);
        let mut skipped = Vec::new();
        let mut queued = 0usize;
        for package in packages {
            // The UI groups ledger rows by repository, but auto-update must
            // visit every recorded executable, including secondary binaries
            // installed from the same release. Latest-release metadata is
            // cached per repository; the attempt guard is per executable.
            let repo_key = package.repo.to_ascii_lowercase();
            let attempt_key = package.path.display().to_string();
            if self.state.auto_attempted.contains(&attempt_key) {
                continue;
            }
            let latest = self
                .state
                .latest
                .get(&repo_key)
                .map(|info| info.release.clone());
            let Some(release) = latest else {
                self.state.auto_attempted.insert(attempt_key);
                skipped.push(format!("{}: latest release unavailable", package.repo));
                continue;
            };
            self.state.auto_attempted.insert(attempt_key);
            match crate::update::decide(&checksum_config, platform, &package, &release) {
                crate::update::Decision::Update(plan) if queued < 32 => {
                    self.state.auto_queue.push_back(*plan);
                    queued += 1;
                }
                crate::update::Decision::Update(_) => {
                    skipped.push(format!("{}: auto-update queue limit reached", package.repo));
                }
                crate::update::Decision::Current => {}
                crate::update::Decision::Skip { reason } => {
                    skipped.push(format!("{}: {reason}", package.repo));
                }
            }
        }
        self.state.auto_skipped = skipped.len();
        if queued > 0 {
            self.start_next_auto_update();
        } else if let Some(first) = skipped.first() {
            self.state.status = Some(Status {
                text: if skipped.len() == 1 {
                    format!("auto-update skipped: {first}")
                } else {
                    format!("auto-update skipped {} package(s): {first}", skipped.len())
                },
                level: StatusLevel::Warning,
            });
        }
    }

    fn start_next_auto_update(&mut self) {
        if self.state.auto_active.is_some() {
            return;
        }
        let Some(plan) = self.state.auto_queue.pop_front() else {
            if self.state.quit_after_auto {
                self.quit = true;
            }
            return;
        };
        let message = format!(
            "auto-updating {} {} → {}",
            plan.package.repo,
            plan.from_version(),
            plan.to_version()
        );
        let command = Command::Install {
            plan: Box::new(plan.plan.clone()),
            config: crate::update::checksum_only_config(self.state.config.clone()),
            auto: true,
        };
        if self.worker.send(command) {
            self.state.auto_active = Some(AutoUpdateActivity {
                plan,
                stage: InstallStage::Downloading,
                current: 0,
                total: None,
            });
            self.state.status = Some(Status {
                text: message,
                level: StatusLevel::Info,
            });
        } else {
            self.state.status = Some(Status {
                text: "network worker stopped during auto-update".to_owned(),
                level: StatusLevel::Error,
            });
            self.state.auto_queue.clear();
            self.state.quit_after_auto = false;
        }
    }

    fn finish_auto_update(&mut self, result: std::result::Result<InstallOutcome, String>) {
        let activity = self.state.auto_active.take();
        if let Some(activity) = activity {
            match result {
                Ok(outcome) => {
                    self.state.auto_updated += 1;
                    self.state.status = Some(Status {
                        text: if outcome.already_current {
                            format!(
                                "auto-update: {} is already current at {}",
                                outcome.repo, outcome.version
                            )
                        } else {
                            format!(
                                "auto-updated {} {} → {}",
                                outcome.repo,
                                activity.plan.from_version(),
                                outcome.version
                            )
                        },
                        level: StatusLevel::Success,
                    });
                }
                Err(error) => {
                    self.state.auto_failed += 1;
                    self.state.status = Some(Status {
                        text: format!(
                            "auto-update failed for {}: {}",
                            activity.plan.package.repo,
                            format::sanitize(&error)
                        ),
                        level: StatusLevel::Warning,
                    });
                }
            }
        }
        self.start_next_auto_update();
        if self.state.auto_active.is_none() && self.state.auto_queue.is_empty() {
            let total = self.state.auto_updated + self.state.auto_failed + self.state.auto_skipped;
            if total > 0 {
                self.state.status = Some(Status {
                    text: format!(
                        "auto-update complete: {} updated · {} failed · {} skipped",
                        self.state.auto_updated, self.state.auto_failed, self.state.auto_skipped
                    ),
                    level: if self.state.auto_failed > 0 {
                        StatusLevel::Warning
                    } else {
                        StatusLevel::Success
                    },
                });
            }
            if self.state.quit_after_auto {
                self.quit = true;
            }
        }
    }

    fn refresh(&mut self) {
        self.state.clear_release();
        self.state.warnings.clear();
        self.state.status = None;
        self.state.latest.clear();
        self.state.latest_errors.clear();
        self.state.auto_queue.clear();
        self.state.auto_active = None;
        self.state.auto_attempted.clear();
        self.state.auto_scan_done = false;
        self.state.auto_updated = 0;
        self.state.auto_failed = 0;
        self.state.auto_skipped = 0;
        self.state.quit_after_auto = false;
        self.state.reload_ledger();
        self.start();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            if self.state.auto_active.is_some() {
                self.state.quit_after_auto = true;
                self.state.auto_queue.clear();
                self.state.status = Some(Status {
                    text: "auto-update is running; harbor will quit after this update".to_owned(),
                    level: StatusLevel::Warning,
                });
            } else {
                self.quit = true;
            }
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
                            auto: false,
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
            KeyCode::Char('q') => {
                if self.state.auto_active.is_some() {
                    self.state.quit_after_auto = true;
                    self.state.auto_queue.clear();
                    self.state.status = Some(Status {
                        text: "auto-update is running; harbor will quit after this update"
                            .to_owned(),
                        level: StatusLevel::Warning,
                    });
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Char('?') => self.state.stage = Stage::Help,
            KeyCode::Char('s') => {
                self.state.source_state.selected = 0;
                self.state.source_state.input = None;
                self.state.source_state.remove_confirm = None;
                self.state.source_state.status = None;
                self.state.stage = Stage::Sources(self.state.source_state.clone());
            }
            KeyCode::Char('r') => {
                if self.state.auto_active.is_some() {
                    self.state.status = Some(Status {
                        text: "auto-update is running; wait before refreshing".to_owned(),
                        level: StatusLevel::Warning,
                    });
                } else {
                    self.refresh();
                }
            }
            KeyCode::Char('/') => {
                self.state.search_buffer = self.state.search_query.clone();
                self.state.search_active = true;
            }
            KeyCode::Char('f') => self.state.cycle_source_filter(),
            KeyCode::Char('v') => self.state.cycle_status_filter(),
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
                        if let Some(key) = self.state.selected_repo_key() {
                            self.state.latest_errors.remove(&key);
                        }
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
        if self.state.auto_active.is_some() {
            self.state.status = Some(Status {
                text: "auto-update is running; wait for it to finish".to_owned(),
                level: StatusLevel::Warning,
            });
            return;
        }
        if let Some(error) = &self.state.ledger_error {
            self.state.status = Some(Status {
                text: format!("install ledger is unavailable: {error}"),
                level: StatusLevel::Error,
            });
            return;
        }
        let Some(release) = self.state.release.as_ref() else {
            self.state.status = Some(Status {
                text: "select a repository and wait for its release to load".to_owned(),
                level: StatusLevel::Warning,
            });
            return;
        };
        let Some(choice) = release.selected_choice() else {
            self.state.status = Some(Status {
                text: format!(
                    "no compatible asset for {}; open the release details to see available files",
                    self.state.platform.label()
                ),
                level: StatusLevel::Error,
            });
            return;
        };
        let installed_version = self
            .state
            .installed_by_repo
            .get(&release.repo.full_name().to_ascii_lowercase())
            .map(|package| package.version.clone());
        let request = InstallRequest {
            repo: release.repo.clone(),
            tag: Some(release.release.tag_name.clone()),
            asset_name: Some(choice.asset.name.clone()),
            asset_glob: None,
            binary: None,
            name: None,
            force: false,
            allow_unverified: false,
            expected_current: None,
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
                    installed_version,
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
