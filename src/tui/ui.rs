use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, Wrap},
};

use super::{
    app::{App, View},
    theme::Theme,
};
use crate::{
    history::{self, date_from_unix},
    snapshot::{self, PackageStats},
};

/// Days shown in the detail view's charts.
const CHART_DAYS: i64 = 30;
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

pub fn draw(f: &mut Frame, app: &mut App, t: &Theme) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(t.bg).fg(t.white)), area);

    let [header, body, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, t, header);
    match (app.view, app.selected().is_some()) {
        (View::Detail, true) => draw_detail(f, app, t, body),
        _ => draw_list(f, app, t, body),
    }
    draw_status(f, app, t, status);
    draw_footer(f, app, t, footer);
    if app.help {
        draw_help(f, t, area);
    }
}

fn ago(secs: i64) -> String {
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

fn opt(n: Option<u64>) -> String {
    n.map_or_else(|| "–".into(), |n| n.to_string())
}

fn block<'a>(title: &'a str, t: &Theme) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.line))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(t.hot_pink).bold(),
        ))
}

/// Short notes for a package row: what needs attention, if anything.
fn notes(p: &PackageStats) -> Vec<(String, bool)> {
    let mut n = Vec::new();
    if p.out_of_date_since.is_some() {
        n.push(("flagged out-of-date".into(), true));
    }
    if let Some(tag) = p.unpackaged_release() {
        n.push((format!("{tag} not packaged"), true));
    }
    if p.maintainer.is_none() {
        n.push(("orphaned".into(), true));
    }
    n
}

fn draw_header(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let mut spans = vec![
        Span::styled(" AURWATCH ", Style::new().fg(t.bg).bg(t.hot_pink).bold()),
        Span::raw(" "),
    ];
    match &app.snap {
        Some(s) => {
            let alerts = s.packages.iter().filter(|p| !notes(p).is_empty()).count();
            spans.extend([
                Span::styled(s.maintainer.clone(), Style::new().fg(t.cyan).bold()),
                Span::styled(
                    format!(
                        "  ·  {} packages  ·  updated {}",
                        s.packages.len(),
                        ago(snapshot::now() - s.fetched_at)
                    ),
                    Style::new().fg(t.muted),
                ),
            ]);
            if alerts > 0 {
                spans.push(Span::styled(
                    format!("  ·  {alerts} need attention"),
                    Style::new().fg(t.red).bold(),
                ));
            }
        }
        None => spans.push(Span::styled("no data yet", Style::new().fg(t.muted))),
    }
    if app.refreshing() {
        spans.push(Span::styled(
            format!("  {} fetching", SPINNER[(app.tick / 2) % SPINNER.len()]),
            Style::new().fg(t.orange),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(t.panel)),
        area,
    );
}

fn draw_list(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let Some(snap) = app.snap.clone() else {
        let msg = if app.refreshing() {
            "Fetching your packages…"
        } else {
            "No data. Press r to fetch."
        };
        f.render_widget(
            Paragraph::new(msg).fg(t.muted).block(block("Packages", t)),
            area,
        );
        return;
    };

    // Tables sized to their rows, stacked at the top; spare space stays below.
    let repo_rows = snap.repos.len() as u16;
    let [pkgs_area, repos_area, _] = Layout::vertical([
        Constraint::Length(snap.packages.len().max(1) as u16 + 3),
        Constraint::Length(if repo_rows > 0 { repo_rows + 3 } else { 0 }),
        Constraint::Fill(1),
    ])
    .areas(area);
    let name_w = snap
        .packages
        .iter()
        .map(|p| p.name.chars().count())
        .max()
        .unwrap_or(7)
        .max(7) as u16
        + 1;

    let head = |cols: &[&'static str]| {
        Row::new(cols.iter().map(|c| Cell::from(*c)).collect::<Vec<_>>())
            .style(Style::new().fg(t.orange).bold())
    };

    let rows: Vec<Row> = snap
        .packages
        .iter()
        .map(|p| {
            let n = notes(p);
            let dot = if n.is_empty() {
                Span::styled("●", Style::new().fg(t.acid_green))
            } else {
                Span::styled("●", Style::new().fg(t.red))
            };
            let d = p.downloads.as_ref();
            let note = n
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Row::new(vec![
                Cell::from(dot),
                Cell::from(Span::styled(p.name.clone(), Style::new().fg(t.cyan))),
                Cell::from(p.version.clone()),
                Cell::from(p.votes.to_string()),
                Cell::from(format!("{:.2}", p.popularity)),
                Cell::from(opt(d.and_then(|d| d.current_release))),
                Cell::from(opt(d.map(|d| d.all_releases))),
                Cell::from(opt(d.and_then(|d| d.last_7d))),
                Cell::from(opt(d.and_then(|d| d.last_30d))),
                Cell::from(Span::styled(note, Style::new().fg(t.red))),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(name_w),
            Constraint::Length(12),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Fill(1),
        ],
    )
    .header(head(&[
        "", "Package", "Version", "Votes", "Pop", "DL cur", "DL all", "7d", "30d", "Notes",
    ]))
    .block(block("Packages", t))
    .row_highlight_style(Style::new().bg(t.panel).add_modifier(Modifier::BOLD))
    .highlight_symbol("▌");
    f.render_stateful_widget(table, pkgs_area, &mut app.table);

    if repo_rows == 0 {
        return;
    }
    let rows: Vec<Row> = snap
        .repos
        .iter()
        .map(|(name, r)| {
            let status = if r.errors.is_empty() {
                Span::styled("ok", Style::new().fg(t.acid_green))
            } else if r.errors.iter().any(|e| e.contains("404")) {
                Span::styled("repo not found", Style::new().fg(t.red))
            } else {
                Span::styled(format!("{} errors", r.errors.len()), Style::new().fg(t.red))
            };
            let pair = |n: Option<u64>, u: Option<u64>| match (n, u) {
                (Some(n), Some(u)) => format!("{n} ({u})"),
                _ => "–".into(),
            };
            Row::new(vec![
                Cell::from(Span::styled(name.clone(), Style::new().fg(t.purple))),
                Cell::from(pair(r.views_14d, r.view_uniques_14d)),
                Cell::from(pair(r.clones_14d, r.clone_uniques_14d)),
                Cell::from(r.views_recorded.to_string()),
                Cell::from(r.clones_recorded.to_string()),
                Cell::from(r.days_recorded.to_string()),
                Cell::from(status),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Fill(1),
        ],
    )
    .header(head(&[
        "GitHub repo",
        "Views 14d",
        "Clones 14d",
        "Views all",
        "Clones all",
        "Days",
        "Status",
    ]))
    .block(block("GitHub traffic (repo, not AUR page) · (unique)", t));
    f.render_widget(table, repos_area);
}

fn draw_detail(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let Some(p) = app.selected() else { return };
    let today = date_from_unix(snapshot::now());
    let [info_area, charts_area] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);

    let label = |k: &'static str| Span::styled(format!("{k:<14}"), Style::new().fg(t.muted));
    // Links get a narrower label column so long URLs fit on one line.
    let link = |k: &'static str| Span::styled(format!("{k:<9}"), Style::new().fg(t.muted));
    let mut lines = vec![
        Line::from(Span::styled(p.name.clone(), Style::new().fg(t.cyan).bold())),
        Line::from(Span::styled(
            p.description.clone().unwrap_or_default(),
            Style::new().fg(t.white),
        )),
        Line::raw(""),
        Line::from(vec![label("Version"), Span::raw(p.version.clone())]),
        Line::from(vec![
            label("Latest tag"),
            match (p.latest_release.as_deref(), p.unpackaged_release()) {
                (_, Some(tag)) => {
                    Span::styled(format!("{tag}  (not packaged)"), Style::new().fg(t.red))
                }
                (Some(tag), None) => {
                    Span::styled(format!("{tag}  (packaged)"), Style::new().fg(t.acid_green))
                }
                (None, None) => Span::styled("–", Style::new().fg(t.muted)),
            },
        ]),
        Line::from(vec![
            label("Maintainer"),
            Span::raw(p.maintainer.clone().unwrap_or_else(|| "orphaned".into())),
        ]),
        Line::from(vec![label("Votes"), Span::raw(p.votes.to_string())]),
        Line::from(vec![
            label("Popularity"),
            Span::raw(format!("{:.4}", p.popularity)),
        ]),
        Line::from(vec![
            label("Flagged"),
            match p.out_of_date_since {
                Some(ts) => Span::styled(
                    format!("out-of-date since {}", date_from_unix(ts)),
                    Style::new().fg(t.red),
                ),
                None => Span::styled("no", Style::new().fg(t.acid_green)),
            },
        ]),
        Line::from(vec![
            label("Submitted"),
            Span::raw(date_from_unix(p.first_submitted)),
        ]),
        Line::from(vec![
            label("Updated"),
            Span::raw(format!(
                "{}  ({})",
                date_from_unix(p.last_modified),
                ago(snapshot::now() - p.last_modified)
            )),
        ]),
        Line::raw(""),
    ];
    if let Some(d) = &p.downloads {
        lines.extend([
            Line::from(Span::styled(
                "Downloads (Linux release assets)",
                Style::new().fg(t.orange).bold(),
            )),
            Line::from(vec![
                label("This release"),
                Span::raw(opt(d.current_release)),
            ]),
            Line::from(vec![
                label("All releases"),
                Span::raw(d.all_releases.to_string()),
            ]),
            Line::from(vec![label("Last 7 days"), Span::raw(opt(d.last_7d))]),
            Line::from(vec![label("Last 30 days"), Span::raw(opt(d.last_30d))]),
        ]);
    } else {
        lines.push(Line::from(Span::styled(
            "Downloads: not counted for source packages (GitHub doesn't count tag archives).",
            Style::new().fg(t.muted),
        )));
    }
    lines.extend([
        Line::raw(""),
        Line::from(vec![
            link("AUR"),
            Span::styled(bare(&p.aur_url), Style::new().fg(t.purple)),
        ]),
        Line::from(vec![
            link("GitHub"),
            Span::styled(
                p.github_repo.clone().unwrap_or_else(|| "–".into()),
                Style::new().fg(t.purple),
            ),
        ]),
        Line::from(vec![
            link("Upstream"),
            Span::styled(
                p.upstream_url.as_deref().map_or_else(|| "–".into(), bare),
                Style::new().fg(t.purple),
            ),
        ]),
    ]);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block("Package", t)),
        info_area,
    );

    // Charts: downloads only for -bin packages; repo traffic when there's a repo.
    let mut charts: Vec<(String, Vec<u64>, ratatui::style::Color)> = Vec::new();
    if p.downloads.is_some() {
        let d = app.hist.downloads_per_day(&p.name, &today, CHART_DAYS);
        charts.push((
            format!(
                "Downloads / day · {CHART_DAYS}d · {}",
                d.iter().sum::<u64>()
            ),
            d,
            t.acid_green,
        ));
    }
    if let Some(repo) = &p.github_repo {
        let (v, c) = app.hist.traffic_per_day(repo, &today, CHART_DAYS);
        charts.push((
            format!(
                "Repo views / day · {CHART_DAYS}d · {}",
                v.iter().sum::<u64>()
            ),
            v,
            t.cyan,
        ));
        charts.push((
            format!(
                "Repo clones / day · {CHART_DAYS}d · {}",
                c.iter().sum::<u64>()
            ),
            c,
            t.purple,
        ));
    }
    if charts.is_empty() {
        f.render_widget(
            Paragraph::new("No GitHub repo, so no charts.")
                .fg(t.muted)
                .block(block("History", t)),
            charts_area,
        );
        return;
    }
    let first_day = history::shift_date(&today, 1 - CHART_DAYS).unwrap_or_default();
    let span_label = format!(
        " {} → {} ",
        first_day.get(5..).unwrap_or(""),
        today.get(5..).unwrap_or("")
    );
    let areas = Layout::vertical(charts.iter().map(|_| Constraint::Fill(1))).split(charts_area);
    for ((title, data, color), a) in charts.iter().zip(areas.iter()) {
        let max = data.iter().copied().max().unwrap_or(0);
        let b = block(title, t)
            .title_bottom(Line::from(Span::styled(
                span_label.clone(),
                Style::new().fg(t.muted),
            )))
            .title_bottom(
                Line::from(Span::styled(
                    format!(" peak {max} "),
                    Style::new().fg(t.muted),
                ))
                .right_aligned(),
            );
        f.render_widget(
            Sparkline::default()
                .block(b)
                .data(stretch(data, a.width.saturating_sub(2)))
                .style(Style::new().fg(*color)),
            *a,
        );
    }
}

/// Repeat each day's value so `data` fills `width` columns (Sparkline draws one bar per value).
fn stretch(data: &[u64], width: u16) -> Vec<u64> {
    let k = (width as usize / data.len().max(1)).max(1);
    data.iter()
        .flat_map(|v| std::iter::repeat_n(*v, k))
        .collect()
}

/// `https://example.com/x/` -> `example.com/x`, for display in narrow panels.
fn bare(url: &str) -> String {
    let u = url.split_once("://").map_or(url, |(_, rest)| rest);
    u.trim_end_matches('/').to_string()
}

fn draw_status(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let color = if app.status_is_error { t.red } else { t.muted };
    f.render_widget(Paragraph::new(format!(" {}", app.status)).fg(color), area);
}

fn draw_footer(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let keys: &[(&str, &str)] = match app.view {
        View::List => &[
            ("↑↓/jk", "move"),
            ("enter", "details"),
            ("r", "refresh"),
            ("o", "AUR"),
            ("g", "GitHub"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Detail => &[
            ("esc", "back"),
            ("↑↓/jk", "prev/next"),
            ("r", "refresh"),
            ("o", "AUR"),
            ("g", "GitHub"),
            ("u", "upstream"),
            ("q", "quit"),
        ],
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in keys {
        spans.push(Span::styled(*k, Style::new().fg(t.orange).bold()));
        spans.push(Span::styled(format!(" {v}   "), Style::new().fg(t.muted)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(t.panel)),
        area,
    );
}

fn draw_help(f: &mut Frame, t: &Theme, area: Rect) {
    let w = 64.min(area.width.saturating_sub(4));
    let h = 20.min(area.height.saturating_sub(2));
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let k = |key: &'static str, what: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {key:<12}"), Style::new().fg(t.orange).bold()),
            Span::raw(what),
        ])
    };
    let lines = vec![
        k("↑↓ j k", "move selection"),
        k("Home End", "first / last package"),
        k("enter l", "package details and 30-day charts"),
        k("esc h", "back to the list"),
        k("r", "fetch now (also runs every 6h via the timer)"),
        k("o / g / u", "open AUR page / GitHub repo / upstream URL"),
        k("q", "quit"),
        Line::raw(""),
        Line::from(Span::styled(
            "  What the numbers mean",
            Style::new().fg(t.hot_pink).bold(),
        )),
        Line::from(Span::styled(
            "  Downloads: GitHub release-asset downloads (Linux only),",
            Style::new().fg(t.muted),
        )),
        Line::from(Span::styled(
            "  -bin packages only. The AUR itself doesn't count downloads.",
            Style::new().fg(t.muted),
        )),
        Line::from(Span::styled(
            "  Views/clones: the GitHub repo's traffic, not the AUR page.",
            Style::new().fg(t.muted),
        )),
        Line::from(Span::styled(
            "  7d/30d and charts fill in as aurwatch builds history.",
            Style::new().fg(t.muted),
        )),
    ];
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(block("Help", t))
            .style(Style::new().bg(t.bg)),
        r,
    );
}
