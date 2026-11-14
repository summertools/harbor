//! Ratatui rendering for the interactive browser.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};

use crate::checksum;
use crate::format;
use crate::ui::{
    ConfirmState, Focus, OutcomeState, ProgressState, ReleaseView, SourceState, Stage,
    StatusFilter, StatusLevel, UiState,
};

const ACCENT: Color = Color::Rgb(94, 234, 212);
const TITLE: Color = Color::Rgb(125, 211, 252);
const DIM: Color = Color::Rgb(148, 163, 184);
const GOOD: Color = Color::Rgb(74, 222, 128);
const WARN: Color = Color::Rgb(251, 191, 36);
const BAD: Color = Color::Rgb(248, 113, 113);
const PANEL_BG: Color = Color::Rgb(15, 23, 42);
const HIGHLIGHT_BG: Color = Color::Rgb(30, 41, 59);

#[derive(Clone, Copy)]
struct Theme {
    color: bool,
}

impl Theme {
    fn fg(self, color: Color) -> Style {
        if self.color {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }

    fn accent(self) -> Style {
        self.fg(ACCENT)
    }

    fn title(self) -> Style {
        self.fg(TITLE).add_modifier(Modifier::BOLD)
    }

    fn dim(self) -> Style {
        self.fg(DIM)
    }

    fn good(self) -> Style {
        self.fg(GOOD)
    }

    fn warn(self) -> Style {
        self.fg(WARN)
    }

    fn bad(self) -> Style {
        self.fg(BAD)
    }

    fn foreground(self) -> Style {
        if self.color {
            Style::default().fg(Color::Rgb(226, 232, 240))
        } else {
            Style::default()
        }
    }

    fn bold(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    fn border(self) -> Style {
        self.fg(DIM)
    }

    fn selected(self) -> Style {
        if self.color {
            Style::default()
                .fg(Color::Rgb(241, 245, 249))
                .bg(HIGHLIGHT_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        }
    }

    fn badge(self) -> Style {
        if self.color {
            Style::default()
                .fg(Color::Rgb(15, 23, 42))
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        }
    }
}

/// Draw one frame.
pub(crate) fn draw(frame: &mut Frame, state: &mut UiState) {
    let theme = Theme { color: state.color };
    let area = frame.area();
    if area.width < 54 || area.height < 14 {
        let lines = vec![
            Line::from(Span::styled("harbor", theme.title())),
            Line::from(""),
            Line::from(Span::styled(
                "terminal too small",
                theme.bad().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("need 54x14, have {}x{}", area.width, area.height),
                theme.dim(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(theme.border()),
            ),
            area,
        );
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(frame, state, outer[0], theme);
    draw_body(frame, state, outer[1], theme);
    draw_footer(frame, state, outer[2], theme);

    match &state.stage {
        Stage::Browse => {}
        Stage::Confirm(confirm) => draw_confirm(frame, confirm, theme),
        Stage::Progress(progress) => draw_progress(frame, progress, theme),
        Stage::Outcome(outcome) => draw_outcome(frame, outcome, theme),
        Stage::Sources(sources) => draw_sources(frame, state, sources, theme),
        Stage::Help => draw_help(frame, theme),
    }
}

fn draw_header(frame: &mut Frame, state: &UiState, area: Rect, theme: Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border())
        .title(Span::styled(" harbor ", theme.badge()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);
    let right_width = 44u16.min(inner.width / 2);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("safe release installer", theme.title()),
            Span::styled("  ·  GitHub releases", theme.dim()),
        ])),
        top[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            state.platform.label(),
            theme.dim(),
        )]))
        .alignment(Alignment::Right),
        top[1],
    );

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(rows[1]);
    let path_style = if state.bin_dir.in_path {
        theme.good()
    } else {
        theme.warn()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("bin ", theme.dim()),
            Span::styled(
                format::truncate(
                    &format::sanitize(&crate::platform::compact_path(&state.bin_dir.path)),
                    usize::from(bottom[0].width.saturating_sub(28)),
                ),
                path_style,
            ),
            Span::styled(
                if state.bin_dir.in_path {
                    "  on PATH"
                } else {
                    "  not on PATH"
                },
                theme.dim(),
            ),
        ])),
        bottom[0],
    );
    let (installed, updates, available) = state.status_counts();
    let counters = format!(
        "{} installed · {} update{} · {} not installed",
        installed,
        updates,
        if updates == 1 { "" } else { "s" },
        available
    );
    let counters = format::truncate(&counters, usize::from(right_width));
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            counters.into_owned(),
            if updates > 0 {
                theme.warn()
            } else {
                theme.dim()
            },
        )]))
        .alignment(Alignment::Right),
        bottom[1],
    );
}

fn draw_body(frame: &mut Frame, state: &mut UiState, area: Rect, theme: Theme) {
    let chunks = if area.width >= 92 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(area)
    };
    if let Some(area) = chunks.first().copied() {
        draw_repo_panel(frame, state, area, theme);
    }
    if let Some(area) = chunks.get(1).copied() {
        draw_detail_panel(frame, state, area, theme);
    }
}

struct StatusChip {
    text: String,
    style: Style,
}

fn status_chip(
    state: &UiState,
    repo: &crate::RepoItem,
    max_width: usize,
    theme: Theme,
) -> StatusChip {
    let installed = state.installed_for(repo);
    let latest = state.latest_for(repo);
    let key = repo.full_name.to_ascii_lowercase();
    let checking = state.status_inflight.contains(&key);
    let full = if let Some(package) = installed {
        if !package.present {
            format!("! missing {}", package.version)
        } else if let Some(latest) = latest {
            if latest.compatible == 0 {
                format!("! {} · no compatible latest", package.version)
            } else if latest.release.tag_name != package.version {
                format!("↑ {} → {}", package.version, latest.release.tag_name)
            } else {
                format!("✓ {}", package.version)
            }
        } else if checking {
            format!("✓ {} …", package.version)
        } else {
            format!("✓ {}", package.version)
        }
    } else if let Some(latest) = latest {
        if latest.compatible == 0 {
            "! no compatible asset".to_owned()
        } else {
            format!("○ not installed · {}", latest.release.tag_name)
        }
    } else if checking {
        "○ checking…".to_owned()
    } else {
        "○ not installed".to_owned()
    };
    let short = if let Some(package) = installed {
        if !package.present {
            format!("! {}", package.version)
        } else if latest.is_some_and(|latest| {
            latest.compatible == 0 || latest.release.tag_name != package.version
        }) {
            format!("↑ {}", package.version)
        } else {
            format!("✓ {}", package.version)
        }
    } else if latest.is_some_and(|latest| latest.compatible == 0) {
        "!".to_owned()
    } else {
        "○".to_owned()
    };
    let warn = installed.is_some_and(|package| !package.present)
        || latest.is_some_and(|latest| latest.compatible == 0)
        || state.has_update(repo);
    let style = if warn {
        theme.warn()
    } else if installed.is_some() {
        theme.good()
    } else {
        theme.dim()
    };
    let text = if format::width(&full) <= max_width {
        full
    } else if format::width(&short) <= max_width {
        short
    } else if installed.is_some() {
        "✓".to_owned()
    } else {
        "○".to_owned()
    };
    StatusChip { text, style }
}

fn install_summary(state: &UiState, theme: Theme) -> Vec<Line<'static>> {
    let Some(repo) = state.selected_repo() else {
        return vec![Line::from(Span::styled(
            "Select a repository to see install status.",
            theme.dim(),
        ))];
    };
    let key = repo.full_name.to_ascii_lowercase();
    let installed = state.installed_for(repo);
    let latest = state.latest_for(repo);
    let checking = state.status_inflight.contains(&key);
    let failed = state.latest_errors.contains_key(&key);
    let mut lines = Vec::new();
    let mut status = vec![Span::styled("status   ", theme.dim())];
    if let Some(package) = installed {
        if package.present {
            status.push(Span::styled("installed ", theme.good()));
        } else {
            status.push(Span::styled("missing ", theme.warn()));
        }
        status.push(Span::styled(package.version.clone(), theme.bold()));
        if let Some(latest) = latest {
            if latest.compatible == 0 {
                status.push(Span::styled(
                    format!(
                        "  ·  latest {} has no compatible asset",
                        latest.release.tag_name
                    ),
                    theme.warn(),
                ));
            } else if latest.release.tag_name != package.version {
                status.push(Span::styled("  →  ", theme.dim()));
                status.push(Span::styled(
                    latest.release.tag_name.clone(),
                    theme.accent(),
                ));
                status.push(Span::styled(" available", theme.warn()));
            } else {
                status.push(Span::styled("  ·  up to date", theme.good()));
            }
        } else if checking {
            status.push(Span::styled("  ·  checking latest…", theme.dim()));
        } else if failed {
            status.push(Span::styled("  ·  latest unavailable", theme.dim()));
        }
    } else {
        status.push(Span::styled("not installed", theme.dim()));
        if let Some(latest) = latest {
            if latest.compatible == 0 {
                status.push(Span::styled(
                    format!(
                        "  ·  latest {} has no compatible asset",
                        latest.release.tag_name
                    ),
                    theme.warn(),
                ));
            } else {
                status.push(Span::styled(
                    format!("  ·  latest {}", latest.release.tag_name),
                    theme.dim(),
                ));
            }
        } else if checking {
            status.push(Span::styled("  ·  checking latest…", theme.dim()));
        }
    }
    lines.push(Line::from(status));
    if let Some(package) = installed {
        let mut path = vec![
            Span::styled("path     ", theme.dim()),
            Span::styled(
                format::sanitize(&package.display_path()),
                if package.present {
                    theme.foreground()
                } else {
                    theme.warn()
                },
            ),
        ];
        if package.binaries > 1 {
            path.push(Span::styled(
                format!("  ·  {} binaries", package.binaries),
                theme.dim(),
            ));
        }
        if !package.present {
            path.push(Span::styled("  ·  file is missing", theme.warn()));
        }
        lines.push(Line::from(path));
        lines.push(Line::from(vec![
            Span::styled("installed ", theme.dim()),
            Span::styled(
                format!("{} UTC", format::unix_datetime(package.installed_at)),
                theme.dim(),
            ),
        ]));
    }
    lines
}

fn draw_repo_panel(frame: &mut Frame, state: &mut UiState, area: Rect, theme: Theme) {
    let filter = state.source_filter_label();
    let title = format!(
        " repositories · {}/{} · {} · {} ",
        state.filtered.len(),
        state.repos.len(),
        state.status_filter.label(),
        filter
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if state.focus == Focus::Repos {
            theme.accent()
        } else {
            theme.border()
        })
        .title(Span::styled(
            format::truncate(&title, usize::from(area.width.saturating_sub(2))).into_owned(),
            theme.title(),
        ));
    if state.loading_repos {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let lines = Text::from(vec![
                Line::from(Span::styled(
                    format!("{} loading sources…", spinner(state.spinner)),
                    theme.accent(),
                )),
                Line::from(Span::styled(
                    "GitHub API requests are running in the background",
                    theme.dim(),
                )),
            ]);
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
        return;
    }
    if state.filtered.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let checking_updates = state.status_filter == StatusFilter::Updates
                && state.repos.iter().any(|repo| {
                    state.installed_for(repo).is_some()
                        && state.latest_for(repo).is_none()
                        && !state
                            .latest_errors
                            .contains_key(&repo.full_name.to_ascii_lowercase())
                });
            let text = if state.repos.is_empty() {
                "No repositories found.\n\nPress s to add an org or repo source."
            } else if checking_updates {
                "Checking installed repositories for updates…"
            } else {
                "No repositories match this filter.\n\nPress / to search, v for status, f for source, Esc to clear."
            };
            frame.render_widget(
                Paragraph::new(Span::styled(text, theme.dim()))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    }
    let inner_width = usize::from(area.width.saturating_sub(2));
    let items: Vec<ListItem<'_>> = state
        .filtered
        .iter()
        .filter_map(|index| state.repos.get(*index))
        .map(|repo| {
            let marker = if repo.installed_only { "◆ " } else { "● " };
            let chip = status_chip(state, repo, inner_width / 3, theme);
            let status = format::sanitize(&chip.text);
            let status_width = format::width(&status);
            let marker_width = 2usize;
            let highlight_width = 2usize;
            let gap = 1usize;
            let content_width =
                inner_width.saturating_sub(marker_width + highlight_width + gap + status_width);
            let source_text = format!("  {}", repo.display_source());
            let mut source_width = format::width(&source_text);
            let mut tags: Vec<(String, Style)> = Vec::new();
            let mut tags_width = 0usize;
            if repo.archived {
                tags.push(("  archived".to_owned(), theme.warn()));
                tags_width += "  archived".len();
            }
            if repo.fork {
                tags.push(("  fork".to_owned(), theme.dim()));
                tags_width += "  fork".len();
            }
            while !tags.is_empty()
                && content_width < 8usize.saturating_add(source_width + tags_width)
            {
                if let Some((text, _)) = tags.pop() {
                    tags_width = tags_width.saturating_sub(text.len());
                }
            }
            if content_width < 8usize.saturating_add(source_width) {
                source_width = 0;
            }
            let name_width = content_width
                .saturating_sub(source_width.saturating_add(tags_width))
                .max(4);
            let clean_name = format::sanitize(&repo.full_name);
            let name = format::truncate(&clean_name, name_width);
            let used = marker_width + format::width(&name) + source_width + tags_width;
            let pad = inner_width.saturating_sub(highlight_width + used + status_width);
            let mut spans = vec![
                Span::styled(
                    marker,
                    if repo.installed_only {
                        theme.title()
                    } else {
                        theme.accent()
                    },
                ),
                Span::styled(name.into_owned(), theme.bold()),
            ];
            if source_width > 0 {
                spans.push(Span::styled(source_text, theme.dim()));
            }
            for (text, style) in tags {
                spans.push(Span::styled(text, style));
            }
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(status, chip.style));
            ListItem::new(Line::from(spans))
        })
        .collect();
    state.repo_state.select(Some(state.selected));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected())
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut state.repo_state);
}

fn draw_detail_panel(frame: &mut Frame, state: &mut UiState, area: Rect, theme: Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if state.focus == Focus::Assets {
            theme.accent()
        } else {
            theme.border()
        })
        .title(Span::styled(" details ", theme.title()));
    let install_lines = install_summary(state, theme);

    if state.release_loading {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let mut lines = install_lines;
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("{} loading latest release…", spinner(state.spinner)),
                theme.accent(),
            )));
            lines.push(Line::from(Span::styled(
                "resolving platform assets and checksum sources",
                theme.dim(),
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
        return;
    }

    if let Some(error) = &state.release_error {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let no_release = error.kind == crate::ui::ReleaseErrorKind::NoRelease;
            let mut lines = install_lines;
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if no_release {
                    "no installable release"
                } else {
                    "release unavailable"
                },
                if no_release {
                    theme.warn().add_modifier(Modifier::BOLD)
                } else {
                    theme.bad().add_modifier(Modifier::BOLD)
                },
            )));
            lines.push(Line::from(Span::styled(
                format::sanitize(&error.message),
                theme.dim(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if no_release {
                    "This repository has no usable release yet. You can still install a specific tag with `harbor install owner/repo --tag ...`, or enable include_prereleases in the config."
                } else {
                    "Press Enter to retry, or check the source with `harbor doctor`."
                },
                theme.dim(),
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
        return;
    }

    let description = state
        .selected_repo()
        .and_then(|repo| repo.description.clone());
    let bin_dir = state.bin_dir.clone();
    let platform = state.platform;
    let Some(release) = state.release.as_mut() else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let mut lines = install_lines;
            if let Some(repo) = state.selected_repo() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format::sanitize(&repo.full_name),
                    theme.title(),
                )));
                lines.push(Line::from(Span::styled(
                    format::sanitize(repo.description.as_deref().unwrap_or("No description.")),
                    theme.dim(),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Press Enter to load the latest release.",
                    theme.dim(),
                )));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
        return;
    };

    let mut summary = install_lines;
    summary.push(Line::from(""));
    summary.extend(release_summary(
        release,
        description.as_deref(),
        &bin_dir,
        platform,
        theme,
    ));
    let inner = block.inner(area);
    let summary_h = u16::try_from(summary.len())
        .unwrap_or(u16::MAX)
        .min(inner.height.saturating_sub(3))
        .max(4);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_h), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(summary).wrap(Wrap { trim: true }), chunks[0]);
    draw_asset_list(frame, release, chunks[1], platform, theme);
}

fn release_summary(
    release: &ReleaseView,
    description: Option<&str>,
    bin_dir: &crate::platform::BinDir,
    platform: crate::platform::Platform,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format::sanitize(&release.repo.full_name()), theme.title()),
        Span::styled("  ·  ", theme.dim()),
        Span::styled(release.release.display_name().to_owned(), theme.bold()),
    ]));
    let description = description
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            release
                .release
                .body
                .as_deref()
                .filter(|body| !body.trim().is_empty())
                .map(|body| body.lines().next().unwrap_or(""))
        })
        .unwrap_or("No description.");
    lines.push(Line::from(Span::styled(
        format::sanitize(description),
        theme.dim(),
    )));
    lines.push(Line::from(vec![
        Span::styled("release ", theme.dim()),
        Span::styled(format::sanitize(&release.release.tag_name), theme.accent()),
        Span::styled(format!("  {}", release.release.date()), theme.dim()),
        if release.release.prerelease {
            Span::styled("  prerelease", theme.warn())
        } else {
            Span::styled("", theme.dim())
        },
    ]));
    if let Some(choice) = release.selected_choice() {
        let checks = checksum::checksum_assets(&release.release, &choice.asset);
        lines.push(Line::from(vec![
            Span::styled("checksum ", theme.dim()),
            if checks.is_empty() {
                Span::styled("not published", theme.warn())
            } else {
                Span::styled("SHA-256 published", theme.good())
            },
            Span::styled(
                format!(
                    "   {}  {}",
                    format::bytes(choice.asset.size),
                    choice.kind.label()
                ),
                theme.dim(),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("assets   ", theme.dim()),
            Span::styled(
                format!("no compatible asset for {}", platform.label()),
                theme.warn(),
            ),
            Span::styled(
                format!("   {} other-platform asset(s)", release.others.len()),
                theme.dim(),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("install ", theme.dim()),
        Span::styled(
            format::sanitize(&bin_dir.display()),
            if bin_dir.in_path {
                theme.good()
            } else {
                theme.warn()
            },
        ),
        Span::styled(
            if bin_dir.in_path {
                "  on PATH"
            } else {
                "  not on PATH"
            },
            theme.dim(),
        ),
    ]));
    lines
}

fn draw_asset_list(
    frame: &mut Frame,
    release: &mut ReleaseView,
    area: Rect,
    platform: crate::platform::Platform,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    let title = if release.choices.is_empty() {
        " assets · no compatible asset ".to_owned()
    } else if release.others.is_empty() {
        format!(" assets · {} compatible ", release.choices.len())
    } else {
        format!(
            " assets · {} compatible · {} other ",
            release.choices.len(),
            release.others.len()
        )
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border())
        .title(Span::styled(title, theme.dim()));
    if release.choices.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        let mut lines = vec![Line::from(Span::styled(
            format!("No compatible asset for {}.", platform.label()),
            theme.warn(),
        ))];
        if release.others.is_empty() {
            lines.push(Line::from(Span::styled(
                "This release publishes no supported archive or binary.",
                theme.dim(),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "This release exists, but its published files target other systems:",
                theme.dim(),
            )));
            lines.push(Line::from(""));
            for choice in release
                .others
                .iter()
                .take(usize::from(inner.height.saturating_sub(4)))
            {
                let reason = crate::release::incompatibility(platform, &choice.asset.name)
                    .unwrap_or("not compatible");
                lines.push(Line::from(vec![
                    Span::styled("  × ", theme.dim()),
                    Span::styled(
                        format::truncate(
                            &format::sanitize(&choice.asset.name),
                            usize::from(inner.width.saturating_sub(22)),
                        )
                        .into_owned(),
                        theme.dim(),
                    ),
                    Span::styled(format!("  {reason}"), theme.warn()),
                ]));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    }
    let inner = block.inner(area);
    let items: Vec<ListItem<'_>> = release
        .choices
        .iter()
        .map(|choice| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<8}", choice.kind.label()), theme.dim()),
                Span::styled(format::sanitize(&choice.asset.name), theme.bold()),
                Span::styled(
                    format!("  {}", format::bytes(choice.asset.size)),
                    theme.dim(),
                ),
            ]))
        })
        .collect();
    release.asset_state.select(Some(release.selected));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected())
        .highlight_symbol("› ");
    // The list is allowed to use the top border row as its title, so render it
    // into the full rectangle rather than the inner area.
    let _ = inner;
    frame.render_stateful_widget(list, area, &mut release.asset_state);
}

fn draw_footer(frame: &mut Frame, state: &UiState, area: Rect, theme: Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let hints = if state.search_active {
        "type to filter  Enter apply  Esc cancel  Ctrl+U clear"
    } else {
        match &state.stage {
            Stage::Browse => match state.focus {
                Focus::Repos => {
                    "↑↓ move  Enter release  i install  Tab assets  / search  v status  f source  s sources  r refresh  ? help  q quit"
                }
                Focus::Assets => {
                    "↑↓ asset  Enter install  i install  Tab repos  Esc back  / search  v status  s sources  ? help  q quit"
                }
            },
            Stage::Confirm(_) => "Enter install  u allow unverified  o overwrite  Esc cancel",
            Stage::Progress(_) => "installation in progress…",
            Stage::Outcome(_) => "Enter back  q quit",
            Stage::Sources(_) => "a add  d remove  Esc close",
            Stage::Help => "Esc close",
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format::truncate(hints, usize::from(rows[0].width)).into_owned(),
            theme.dim(),
        ))),
        rows[0],
    );

    let line = if state.search_active {
        Line::from(vec![
            Span::styled("search ", theme.title()),
            Span::styled(format::sanitize(&state.search_buffer), theme.bold()),
            Span::styled("_", theme.accent()),
        ])
    } else if let Some(activity) = state.auto_activity_line() {
        Line::from(Span::styled(format::sanitize(&activity), theme.accent()))
    } else if let Some(status) = &state.status {
        let style = match status.level {
            StatusLevel::Info => theme.accent(),
            StatusLevel::Success => theme.good(),
            StatusLevel::Warning => theme.warn(),
            StatusLevel::Error => theme.bad(),
        };
        Line::from(Span::styled(format::sanitize(&status.text), style))
    } else if let Some(warning) = state.warnings.first() {
        Line::from(Span::styled(format::sanitize(warning), theme.warn()))
    } else if let Some(error) = &state.ledger_error {
        Line::from(Span::styled(
            format!("install ledger: {}", format::sanitize(error)),
            theme.warn(),
        ))
    } else {
        Line::from(vec![
            Span::styled("status ", theme.dim()),
            Span::styled(state.status_filter.label(), theme.accent()),
            Span::styled("   source ", theme.dim()),
            Span::styled(state.source_filter_label(), theme.accent()),
            Span::styled("   ", theme.dim()),
            Span::styled(
                if state.search_query.is_empty() {
                    String::new()
                } else {
                    format!("filter {:?}", state.search_query)
                },
                theme.dim(),
            ),
            Span::styled(
                if state.config.auto_update {
                    "   auto-update on"
                } else {
                    ""
                },
                theme.good(),
            ),
        ])
    };
    frame.render_widget(
        Paragraph::new(line)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

fn modal(frame: &mut Frame, width_percent: u16, height: u16, title: &str, theme: Theme) -> Rect {
    let area = frame.area();
    let width = area
        .width
        .saturating_mul(width_percent)
        .saturating_div(100)
        .clamp(40, area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2)).max(6);
    let rect = center_rect(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.accent())
        .style(if theme.color {
            Style::default().bg(PANEL_BG)
        } else {
            Style::default()
        })
        .title(Span::styled(format!(" {title} "), theme.title()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

fn center_rect(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn draw_confirm(frame: &mut Frame, confirm: &ConfirmState, theme: Theme) {
    let plan = &confirm.plan;
    let action = match &confirm.installed_version {
        None => "confirm install",
        Some(current) if current == &plan.release.tag_name => "confirm reinstall",
        Some(_) => "confirm update",
    };
    let inner = modal(frame, 76, 16, action, theme);
    if inner.is_empty() {
        return;
    }
    let mut lines = vec![
        Line::from(vec![
            Span::styled("repository ", theme.dim()),
            Span::styled(plan.request.repo.full_name(), theme.title()),
        ]),
        Line::from(vec![
            Span::styled("release    ", theme.dim()),
            Span::styled(format::sanitize(&plan.release.tag_name), theme.accent()),
            Span::styled(format!("  ({})", plan.release.date()), theme.dim()),
        ]),
    ];
    if let Some(current) = &confirm.installed_version {
        lines.push(Line::from(vec![
            Span::styled("installed  ", theme.dim()),
            Span::styled(format::sanitize(current), theme.good()),
            if current == &plan.release.tag_name {
                Span::styled("  ·  same version (reinstall)", theme.dim())
            } else {
                Span::styled(
                    format!("  →  {}", format::sanitize(&plan.release.tag_name)),
                    theme.warn(),
                )
            },
        ]));
    }
    lines.extend(vec![
        Line::from(vec![
            Span::styled("asset      ", theme.dim()),
            Span::styled(
                format::truncate(
                    &format::sanitize(&plan.choice.asset.name),
                    usize::from(inner.width.saturating_sub(12)),
                )
                .into_owned(),
                theme.bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("checksum   ", theme.dim()),
            if plan.checksum_available() {
                Span::styled("SHA-256 published", theme.good())
            } else {
                Span::styled("not published", theme.warn())
            },
        ]),
        Line::from(vec![
            Span::styled("install dir ", theme.dim()),
            Span::styled(
                format::truncate(
                    &format::sanitize(&plan.bin_dir.display().to_string()),
                    usize::from(inner.width.saturating_sub(12)),
                )
                .into_owned(),
                theme.bold(),
            ),
        ]),
        Line::from(""),
    ]);
    if !plan.checksum_available() {
        lines.push(Line::from(Span::styled(
            if confirm.allow_unverified {
                "! unverified install enabled (u to revoke)"
            } else {
                "! no SHA-256 checksum; press u to allow an unverified install"
            },
            theme.warn(),
        )));
    } else if confirm.allow_unverified {
        lines.push(Line::from(Span::styled(
            "! unverified fallback enabled; a published checksum is still verified when usable",
            theme.warn(),
        )));
    }
    if confirm.force {
        lines.push(Line::from(Span::styled(
            "! overwrite unmanaged file enabled (o to revoke)",
            theme.warn(),
        )));
    }
    if let Some(warning) = &plan.warning {
        lines.push(Line::from(Span::styled(
            format::sanitize(warning),
            theme.dim(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter/y install    u unverified    o overwrite    Esc cancel",
        theme.dim(),
    )));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_progress(frame: &mut Frame, progress: &ProgressState, theme: Theme) {
    let inner = modal(frame, 68, 10, "installing", theme);
    if inner.is_empty() {
        return;
    }
    let ratio = match progress.total.filter(|total| *total > 0) {
        Some(total) => (progress.current as f64 / total as f64).clamp(0.0, 1.0),
        None => 0.0,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    let label = match progress.total.filter(|total| *total > 0) {
        Some(total) => format!(
            "{}  {:.0}%  {}/{}",
            progress.stage.label(),
            ratio * 100.0,
            format::bytes(progress.current),
            format::bytes(total)
        ),
        None => format!("{}  {}", progress.stage.label(), spinner(0)),
    };
    frame.render_widget(
        Gauge::default()
            .gauge_style(theme.accent())
            .ratio(ratio)
            .label(Span::styled(label, theme.bold())),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "→ {}",
                format::sanitize(&progress.plan.bin_dir.display().to_string())
            ),
            theme.dim(),
        ))),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Downloading and verifying in the background. Please wait.",
            theme.dim(),
        ))),
        chunks[2],
    );
}

fn draw_outcome(frame: &mut Frame, outcome: &OutcomeState, theme: Theme) {
    match &outcome.result {
        Ok(result) => {
            let inner = modal(frame, 76, 14, "installed", theme);
            if inner.is_empty() {
                return;
            }
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("repository ", theme.dim()),
                    Span::styled(result.repo.clone(), theme.title()),
                ]),
                Line::from(vec![
                    Span::styled("version    ", theme.dim()),
                    Span::styled(format::sanitize(&result.version), theme.accent()),
                ]),
                Line::from(vec![
                    Span::styled("path       ", theme.dim()),
                    Span::styled(
                        format::sanitize(&result.path.display().to_string()),
                        theme.bold(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("checksum   ", theme.dim()),
                    if result.checksum_verified {
                        Span::styled("SHA-256 verified", theme.good())
                    } else {
                        Span::styled("not verified", theme.warn())
                    },
                ]),
                Line::from(vec![
                    Span::styled("size       ", theme.dim()),
                    Span::styled(format::bytes(result.bytes), theme.dim()),
                ]),
            ];
            if result.already_current {
                lines.push(Line::from(Span::styled(
                    "already up to date; nothing was replaced",
                    theme.good(),
                )));
            }
            if let Some(warning) = &result.warning {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format::sanitize(warning),
                    theme.warn(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Press Enter to return, q to quit.",
                theme.dim(),
            )));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
        Err(error) => {
            let inner = modal(frame, 76, 14, "install failed", theme);
            if inner.is_empty() {
                return;
            }
            let lines = vec![
                Line::from(Span::styled("The install did not complete.", theme.bad())),
                Line::from(""),
                Line::from(Span::styled(format::sanitize(error), theme.dim())),
                Line::from(""),
                Line::from(Span::styled(
                    "Nothing was replaced unless the message above says otherwise.",
                    theme.dim(),
                )),
                Line::from(Span::styled(
                    "Press Enter to return, q to quit.",
                    theme.dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }
    }
}

fn draw_sources(frame: &mut Frame, state: &UiState, sources: &SourceState, theme: Theme) {
    let inner = modal(frame, 74, 18, "sources", theme);
    if inner.is_empty() {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(inner);
    let items: Vec<ListItem<'_>> = state
        .config
        .sources
        .iter()
        .map(|source| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<6}", source.kind.label()), theme.dim()),
                Span::styled(source.name.clone(), theme.bold()),
            ]))
        })
        .collect();
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(if state.config.sources.is_empty() {
        None
    } else {
        Some(sources.selected.min(state.config.sources.len() - 1))
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(theme.border())
                    .title(Span::styled(" configured ", theme.dim())),
            )
            .highlight_style(theme.selected())
            .highlight_symbol("› "),
        chunks[0],
        &mut list_state,
    );
    let mut lines = Vec::new();
    if let Some(input) = &sources.input {
        lines.push(Line::from(vec![
            Span::styled("new source: ", theme.dim()),
            Span::styled(format::sanitize(input), theme.bold()),
            Span::styled("_", theme.accent()),
        ]));
        lines.push(Line::from(Span::styled(
            "org:name, repo:owner/name, or owner/repo · Enter save · Esc cancel",
            theme.dim(),
        )));
    } else if let Some(index) = sources.remove_confirm {
        let name = state
            .config
            .sources
            .get(index)
            .map_or_else(|| "source".to_owned(), |source| source.label());
        lines.push(Line::from(Span::styled(
            format!("Remove {name}? [y/N]"),
            theme.warn(),
        )));
    } else if let Some(status) = &sources.status {
        let style = match status.level {
            StatusLevel::Success => theme.good(),
            StatusLevel::Warning => theme.warn(),
            StatusLevel::Error => theme.bad(),
            StatusLevel::Info => theme.accent(),
        };
        lines.push(Line::from(Span::styled(
            format::sanitize(&status.text),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "config: {}",
                format::sanitize(&crate::platform::compact_path(&state.config_path))
            ),
            theme.dim(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "config: {}",
                format::sanitize(&crate::platform::compact_path(&state.config_path))
            ),
            theme.dim(),
        )));
        lines.push(Line::from(Span::styled(
            "a add source   d remove   Esc close",
            theme.dim(),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), chunks[1]);
}

fn draw_help(frame: &mut Frame, theme: Theme) {
    let inner = modal(frame, 78, 22, "help", theme);
    if inner.is_empty() {
        return;
    }
    let lines = vec![
        Line::from(Span::styled("navigation", theme.title())),
        Line::from(Span::styled(
            "  ↑/k, ↓/j move     PgUp/PgDn page     Home/End first/last",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  Tab switch pane    Enter inspect/install    Esc back",
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled("browsing", theme.title())),
        Line::from(Span::styled(
            "  / search   v status   f source   s sources   r refresh   ? help   q quit",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  v cycles: all / installed / updates / not installed",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  rows show installed version, update availability and missing files",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  i install, update or reinstall the selected asset",
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled("safety", theme.title())),
        Line::from(Span::styled(
            "  HTTPS GitHub hosts only · SHA-256 verified when published",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  archives reject links/devices/.. · atomic replace · ledger protects files",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "  existing unmanaged files require explicit overwrite in the confirm screen",
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled("sources", theme.title())),
        Line::from(Span::styled(
            "  Default: org:summertools. Add any org or owner/repo with `s`.",
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled("Press Esc to close.", theme.dim())),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn spinner(index: usize) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[index % FRAMES.len()]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::config::Config;
    use crate::github::{Asset, Release, RepoRef};
    use crate::install::{InstallOutcome, InstalledPackage, Ledger};
    use crate::platform::{BinDir, Platform};
    use crate::release::{AssetChoice, AssetKind};
    use crate::ui::{
        Focus, LatestInfo, OutcomeState, ReleaseError, ReleaseErrorKind, Stage, StatusFilter,
        UiState,
    };

    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            id: 1,
            name: name.to_owned(),
            size: 1024 * 1024,
            download_count: None,
            content_type: None,
            browser_download_url: format!("https://github.com/o/r/releases/download/v1/{name}"),
        }
    }

    fn release() -> Release {
        Release {
            tag_name: "v1.2.3".to_owned(),
            name: Some("Harbor demo".to_owned()),
            body: Some("A demo release for layout tests.".to_owned()),
            draft: false,
            prerelease: false,
            published_at: Some("2026-11-11T08:04:21Z".to_owned()),
            assets: vec![asset("tool-v1.2.3-x86_64-unknown-linux-musl.tar.gz")],
        }
    }

    fn state() -> UiState {
        let config = Config::default();
        let mut state = UiState::new(
            config,
            Platform::current(),
            BinDir {
                path: PathBuf::from("/tmp/harbor-bin"),
                in_path: true,
            },
            false,
            PathBuf::from("/tmp/harbor-config.toml"),
        );
        state.loading_repos = false;
        state.ledger = Ledger::default();
        state.ledger_error = None;
        state.latest_errors.clear();
        state.status_inflight.clear();
        state.repos = vec![
            crate::RepoItem {
                source: crate::Source::org("summertools").unwrap(),
                full_name: "summertools/rtop".to_owned(),
                name: "rtop".to_owned(),
                description: Some("A terminal system monitor".to_owned()),
                archived: false,
                fork: false,
                html_url: None,
                installed_only: false,
            },
            crate::RepoItem {
                source: crate::Source::org("summertools").unwrap(),
                full_name: "summertools/dulens".to_owned(),
                name: "dulens".to_owned(),
                description: Some("A disk usage explorer".to_owned()),
                archived: false,
                fork: false,
                html_url: None,
                installed_only: false,
            },
        ];
        let package = InstalledPackage {
            repo: "summertools/rtop".to_owned(),
            version: "v1.0.0".to_owned(),
            path: PathBuf::from("/tmp/harbor-bin/rtop"),
            bin_dir: PathBuf::from("/tmp/harbor-bin"),
            installed_at: 1_700_000_000,
            binaries: 1,
            present: true,
        };
        state.installed = vec![package.clone()];
        state.installed_by_repo.clear();
        state
            .installed_by_repo
            .insert("summertools/rtop".to_owned(), package);
        state.latest.insert(
            "summertools/rtop".to_owned(),
            LatestInfo {
                release: release(),
                compatible: 1,
            },
        );
        state.rebuild_filter();
        let choice = AssetChoice {
            asset: asset("rtop-v1.2.3-x86_64-unknown-linux-musl.tar.gz"),
            kind: AssetKind::TarGz,
            score: 100,
            explicit: false,
        };
        state.release = Some(ReleaseView::new(
            RepoRef {
                owner: "summertools".to_owned(),
                name: "rtop".to_owned(),
            },
            release(),
            crate::release::AssetCatalog {
                compatible: vec![choice],
                incompatible: Vec::new(),
            },
        ));
        state.focus = Focus::Repos;
        state
    }

    fn render(state: &mut UiState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn untrusted_text_is_sanitized_in_every_pane() {
        let mut state = state();
        state.repos[0].description = Some("bad\u{1b}[31m\u{202e}\nline".to_owned());
        if let Some(release) = state.release.as_mut() {
            release.release.body = Some("body\u{1b}]0;title\u{7}".to_owned());
            release.choices[0].asset.name = "tool\u{202e}.tar.gz".to_owned();
        }
        state.release_error = Some(crate::ui::ReleaseError {
            message: "error\u{1b}[31m".to_owned(),
            kind: crate::ui::ReleaseErrorKind::Unavailable,
        });
        let text = render(&mut state, 100, 30);
        assert!(!text.contains('\u{1b}'), "escape leaked: {text:?}");
        assert!(!text.contains('\u{202e}'), "bidi leaked: {text:?}");
    }

    #[test]
    fn tiny_terminals_show_the_size_hint() {
        let mut state = state();
        let text = render(&mut state, 40, 10);
        assert!(text.contains("terminal too small"));
    }

    #[test]
    fn source_and_progress_modals_render() {
        let mut state = state();
        state.stage = Stage::Sources(SourceState {
            selected: 0,
            input: Some("repo:owner/tool".to_owned()),
            remove_confirm: None,
            status: Some(crate::ui::Status {
                text: "added repo:owner/tool".to_owned(),
                level: StatusLevel::Success,
            }),
        });
        let sources = render(&mut state, 100, 30);
        assert!(sources.contains("sources"));
        assert!(sources.contains("new source"));

        let plan = crate::install::build_plan(
            &state.config,
            state.platform,
            &state.bin_dir,
            crate::install::InstallRequest {
                repo: RepoRef {
                    owner: "summertools".to_owned(),
                    name: "rtop".to_owned(),
                },
                tag: Some("v1.2.3".to_owned()),
                asset_name: Some("rtop-v1.2.3-x86_64-unknown-linux-musl.tar.gz".to_owned()),
                asset_glob: None,
                binary: None,
                name: None,
                force: false,
                allow_unverified: false,
                expected_current: None,
            },
            release(),
            AssetChoice {
                asset: asset("rtop-v1.2.3-x86_64-unknown-linux-musl.tar.gz"),
                kind: AssetKind::TarGz,
                score: 100,
                explicit: false,
            },
        )
        .unwrap();
        state.stage = Stage::Progress(crate::ui::ProgressState {
            plan,
            stage: crate::install::InstallStage::Downloading,
            current: 50,
            total: Some(100),
        });
        let progress = render(&mut state, 100, 30);
        assert!(progress.contains("installing"));
    }

    #[test]
    fn status_filters_and_badges_show_installed_and_available() {
        let mut state = state();
        let text = render(&mut state, 120, 34);
        assert!(text.contains("↑ v1.0.0"), "update badge missing: {text}");
        assert!(
            text.contains("not installed"),
            "available badge missing: {text}"
        );
        assert!(text.contains("1 installed"), "header count missing: {text}");

        state.status_filter = StatusFilter::Installed;
        state.rebuild_filter();
        assert_eq!(state.filtered.len(), 1);
        assert_eq!(state.selected_repo().unwrap().full_name, "summertools/rtop");

        state.status_filter = StatusFilter::Updates;
        state.rebuild_filter();
        assert_eq!(state.filtered.len(), 1);

        state.status_filter = StatusFilter::NotInstalled;
        state.rebuild_filter();
        assert_eq!(state.filtered.len(), 1);
        assert_eq!(
            state.selected_repo().unwrap().full_name,
            "summertools/dulens"
        );
    }

    #[test]
    fn incompatible_release_assets_are_explained_not_treated_as_load_failure() {
        let mut state = state();
        let foreign = if cfg!(target_os = "windows") {
            "rtop-linux-x86_64.tar.gz"
        } else {
            "rtop-windows-x86_64.zip"
        };
        let choice = AssetChoice {
            asset: asset(foreign),
            kind: if foreign.ends_with(".zip") {
                AssetKind::Zip
            } else {
                AssetKind::TarGz
            },
            score: -100,
            explicit: false,
        };
        state.release = Some(ReleaseView::new(
            RepoRef {
                owner: "summertools".to_owned(),
                name: "rtop".to_owned(),
            },
            release(),
            crate::release::AssetCatalog {
                compatible: Vec::new(),
                incompatible: vec![choice],
            },
        ));
        state.release_error = None;
        let text = render(&mut state, 120, 36);
        assert!(text.contains("No compatible asset"), "{text}");
        assert!(text.contains("different operating"), "{text}");
        assert!(!text.contains("release unavailable"), "{text}");
    }

    #[test]
    fn missing_release_uses_a_distinct_message() {
        let mut state = state();
        state.release = None;
        state.release_error = Some(ReleaseError {
            message: "no stable release".to_owned(),
            kind: ReleaseErrorKind::NoRelease,
        });
        let text = render(&mut state, 100, 30);
        assert!(text.contains("no installable release"), "{text}");
        assert!(!text.contains("release unavailable"), "{text}");
    }

    #[test]
    fn common_terminal_sizes_render_without_panicking() {
        let mut state = state();
        for (width, height) in [(54, 14), (80, 24), (120, 36), (200, 60)] {
            let text = render(&mut state, width, height);
            assert!(
                text.contains("harbor"),
                "missing header at {width}x{height}"
            );
            assert!(
                text.contains("rtop"),
                "missing repository at {width}x{height}"
            );
        }
    }

    #[test]
    fn both_side_by_side_and_stacked_layouts_work() {
        let mut state = state();
        let wide = render(&mut state, 120, 36);
        assert!(wide.contains("repositories"));
        assert!(wide.contains("assets"));
        let narrow = render(&mut state, 60, 30);
        assert!(narrow.contains("repositories"));
    }

    #[test]
    fn modal_stages_fit_and_show_actions() {
        let mut state = state();
        let plan = crate::install::build_plan(
            &state.config,
            state.platform,
            &state.bin_dir,
            crate::install::InstallRequest {
                repo: RepoRef {
                    owner: "summertools".to_owned(),
                    name: "rtop".to_owned(),
                },
                tag: Some("v1.2.3".to_owned()),
                asset_name: Some("rtop-v1.2.3-x86_64-unknown-linux-musl.tar.gz".to_owned()),
                asset_glob: None,
                binary: None,
                name: None,
                force: false,
                allow_unverified: false,
                expected_current: None,
            },
            release(),
            AssetChoice {
                asset: asset("rtop-v1.2.3-x86_64-unknown-linux-musl.tar.gz"),
                kind: AssetKind::TarGz,
                score: 100,
                explicit: false,
            },
        )
        .unwrap();
        state.stage = Stage::Confirm(ConfirmState {
            plan,
            allow_unverified: false,
            force: false,
            installed_version: None,
        });
        let confirm = render(&mut state, 100, 34);
        assert!(confirm.contains("confirm install"));
        assert!(confirm.contains("Enter"));

        state.stage = Stage::Outcome(OutcomeState {
            result: Ok(InstallOutcome {
                path: PathBuf::from("/tmp/harbor-bin/rtop"),
                repo: "summertools/rtop".to_owned(),
                version: "v1.2.3".to_owned(),
                asset: "rtop.tar.gz".to_owned(),
                sha256: "a".repeat(64),
                bytes: 1024,
                checksum_verified: true,
                already_current: false,
                warning: None,
            }),
        });
        let outcome = render(&mut state, 100, 34);
        assert!(outcome.contains("installed"));
    }
}
