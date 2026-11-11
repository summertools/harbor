//! Ratatui rendering for the interactive browser.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};

use crate::checksum;
use crate::format;
use crate::ui::{
    ConfirmState, Focus, OutcomeState, ProgressState, ReleaseView, SourceState, Stage, StatusLevel,
    UiState,
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
            Constraint::Length(3),
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
    let right_width = 30u16.min(inner.width / 2);
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "{} source{} · {} repo{}",
                state.config.sources.len(),
                if state.config.sources.len() == 1 {
                    ""
                } else {
                    "s"
                },
                state.repos.len(),
                if state.repos.len() == 1 { "" } else { "s" },
            ),
            theme.dim(),
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

fn draw_repo_panel(frame: &mut Frame, state: &mut UiState, area: Rect, theme: Theme) {
    let filter = state.source_filter_label();
    let title = format!(
        " repositories · {}/{} · {} ",
        state.filtered.len(),
        state.repos.len(),
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
            let text = if state.repos.is_empty() {
                "No repositories found.\n\nPress s to add an org or repo source."
            } else {
                "No repositories match this filter.\n\nPress / to search, f to change source, Esc to clear."
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
    let items: Vec<ListItem<'_>> = state
        .filtered
        .iter()
        .filter_map(|index| state.repos.get(*index))
        .map(|repo| {
            let mut spans = vec![
                Span::styled("● ", theme.accent()),
                Span::styled(format::sanitize(&repo.full_name), theme.bold()),
                Span::styled(format!("  {}", repo.source.label()), theme.dim()),
            ];
            if repo.archived {
                spans.push(Span::styled("  archived", theme.warn()));
            }
            if repo.fork {
                spans.push(Span::styled("  fork", theme.dim()));
            }
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
    if state.release_loading {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        format!("{} loading latest release…", spinner(state.spinner)),
                        theme.accent(),
                    )),
                    Line::from(Span::styled(
                        "resolving platform assets and checksum sources",
                        theme.dim(),
                    )),
                ])
                .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    }
    if let Some(error) = &state.release_error {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled("could not load release", theme.bad())),
                    Line::from(""),
                    Line::from(Span::styled(format::sanitize(error), theme.dim())),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press r to retry, or check the source with `harbor doctor`.",
                        theme.dim(),
                    )),
                ])
                .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    }
    let description = state
        .selected_repo()
        .and_then(|repo| repo.description.clone());
    let bin_dir = state.bin_dir.clone();
    let Some(release) = state.release.as_mut() else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if !inner.is_empty() {
            let text = if let Some(repo) = state.selected_repo() {
                format!(
                    "{}\n\n{}\n\nPress Enter to load the latest release.",
                    repo.full_name,
                    repo.description.as_deref().unwrap_or("No description.")
                )
            } else {
                "Select a repository to inspect its latest release.".to_owned()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(format::sanitize(&text), theme.dim()))
                    .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    };
    let summary = release_summary(release, description.as_deref(), &bin_dir, theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
        .split(block.inner(area));
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(summary).wrap(Wrap { trim: true }), chunks[0]);
    draw_asset_list(frame, release, chunks[1], theme);
}

fn release_summary(
    release: &ReleaseView,
    description: Option<&str>,
    bin_dir: &crate::platform::BinDir,
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

fn draw_asset_list(frame: &mut Frame, release: &mut ReleaseView, area: Rect, theme: Theme) {
    if area.is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border())
        .title(Span::styled(
            format!(" assets · {} compatible ", release.choices.len()),
            theme.dim(),
        ));
    if release.choices.is_empty() {
        frame.render_widget(block, area);
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
                    "↑↓ move  Enter release  i install  Tab assets  / search  f source  s sources  r refresh  ? help  q quit"
                }
                Focus::Assets => {
                    "↑↓ asset  Enter install  i install  Tab repos  Esc back  / search  s sources  ? help  q quit"
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
    } else {
        Line::from(vec![
            Span::styled("source ", theme.dim()),
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
    let inner = modal(frame, 76, 15, "confirm install", theme);
    if inner.is_empty() {
        return;
    }
    let plan = &confirm.plan;
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
    ];
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
            "  / search   f cycle source   s sources   r refresh   ? help   q quit",
            theme.dim(),
        )),
        Line::from(Span::styled("  i install the selected asset", theme.dim())),
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
    use crate::install::InstallOutcome;
    use crate::platform::{BinDir, Platform};
    use crate::release::{AssetChoice, AssetKind};
    use crate::ui::{Focus, OutcomeState, Stage, UiState};

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
        state.repos = vec![crate::RepoItem {
            source: crate::Source::org("summertools").unwrap(),
            full_name: "summertools/rtop".to_owned(),
            name: "rtop".to_owned(),
            description: Some("A terminal system monitor".to_owned()),
            archived: false,
            fork: false,
            html_url: None,
        }];
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
            vec![choice],
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
        state.release_error = Some("error\u{1b}[31m".to_owned());
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
