// SPDX-License-Identifier: GPL-3.0-or-later
//! Dashboard: one card per UPS with status, dual bars (load + battery),
//! a stats line, and a load sparkline. Compact fallback when space is tight.

use super::widgets::{block, labeled_bar};
use crate::app::App;
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::symbols;
use ratatui::widgets::{Axis, Chart, Dataset, GraphType, Paragraph};
use ratatui::Frame;

const CARD_MIN_H: u16 = 7;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.upses.is_empty() {
        frame.render_widget(Paragraph::new(" no UPS instances configured"), area);
        return;
    }
    let n = app.upses.len() as u16;
    if area.height / n < CARD_MIN_H {
        draw_compact(frame, area, app, theme);
        return;
    }
    let rows = Layout::vertical(
        (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect::<Vec<_>>(),
    )
    .split(area);
    for (i, row) in rows.iter().enumerate() {
        draw_card(frame, *row, app, i, theme);
    }
}

fn fmt_opt(v: Option<f64>, suffix: &str) -> String {
    match v {
        Some(x) => format!("{x:.1}{suffix}"),
        None => format!("--{suffix}"),
    }
}

fn draw_card(frame: &mut Frame, area: Rect, app: &App, idx: usize, theme: &Theme) {
    let panel = &app.upses[idx];
    let selected = idx == app.selected;
    let comm_lost = panel.error.is_some();
    let status_text = if comm_lost {
        "COMMLOST".to_string()
    } else {
        panel.status.as_ref().map(|s| s.status_text().to_string())
            .unwrap_or_else(|| "CONNECTING".to_string())
    };
    let on_battery = status_text.contains("ONBATT");
    let model = panel.status.as_ref().and_then(|s| s.get("MODEL")).unwrap_or("").to_string();

    let border_style = if selected {
        Style::default().fg(theme.accent())
    } else {
        Style::default().fg(theme.dim())
    };

    let rail = if selected { if theme.ascii { ">" } else { "▌" } } else { " " };
    let title = Line::from(vec![
        Span::styled(rail, Style::default().fg(theme.accent())),
        Span::styled(
            format!(" {} ", panel.name),
            Style::default().add_modifier(Modifier::BOLD).fg(if selected { theme.accent() } else { theme.fg() }),
        ),
        Span::styled(format!("{model} "), Style::default().fg(theme.dim())),
        Span::styled(
            format!("{} ", theme.flow_glyph(on_battery, comm_lost)),
            Style::default().fg(theme.status(&status_text)),
        ),
        Span::styled(
            format!("{status_text} "),
            Style::default().fg(theme.status(&status_text)).add_modifier(Modifier::BOLD),
        ),
    ]);

    let blk = block(theme).border_style(border_style).title(title);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.height < 3 || inner.width < 24 {
        return;
    }

    let [bars_area, stats_area, spark_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(inner);

    // dual bars: load (rising palette) + battery (falling palette)
    let (load, charge) = match &panel.status {
        Some(s) => (s.num("LOADPCT"), s.num("BCHARGE")),
        None => (None, None),
    };
    let bar_w = ((bars_area.width as usize).saturating_sub(28) / 2).max(6);
    let mut bar_spans = labeled_bar(theme, "load", load, bar_w, theme.rising(load.unwrap_or(0.0)));
    bar_spans.push(Span::raw("   "));
    bar_spans.extend(labeled_bar(theme, "batt", charge, bar_w, theme.falling(charge.unwrap_or(0.0))));
    frame.render_widget(Paragraph::new(Line::from(bar_spans)), bars_area);

    // stats line
    let stats = if let Some(err) = &panel.error {
        Line::from(Span::styled(
            format!(" {} {}", panel.addr, truncate(err, stats_area.width as usize)),
            Style::default().fg(theme.error_color()),
        ))
    } else if let Some(s) = &panel.status {
        let mut spans = vec![
            kv(theme, "line", fmt_opt(s.num("LINEV"), "V")),
            kv(theme, "batt", fmt_opt(s.num("BATTV"), "V")),
            kv(theme, "out", fmt_opt(s.watts(), "W")),
            kv(theme, "run", fmt_opt(s.num("TIMELEFT"), "m")),
            kv(theme, "xfers", s.get("NUMXFERS").unwrap_or("--").to_string()),
        ]
        .concat();
        if !theme.ascii {
            let load_now = s.num("LOADPCT").unwrap_or(0.0);
            spans.push(Span::styled("    ▮", Style::default().fg(theme.rising(load_now))));
            spans.push(Span::styled("load ", Style::default().fg(theme.dim())));
            spans.push(Span::styled("⠉", Style::default().fg(theme.accent())));
            spans.push(Span::styled("batt", Style::default().fg(theme.dim())));
        }
        Line::from(spans)
    } else {
        Line::from(Span::styled(
            format!(" connecting to {} ...", panel.addr),
            Style::default().fg(theme.dim()),
        ))
    };
    frame.render_widget(Paragraph::new(stats), stats_area);

    draw_history(frame, spark_area, panel, theme);
}

fn kv(theme: &Theme, k: &str, v: String) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {k} "), Style::default().fg(theme.dim())),
        Span::raw(v),
    ]
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "..."
    }
}

/// Dual history plot: load as bars, battery charge as a line, both 0-100%.
fn draw_history(frame: &mut Frame, area: Rect, panel: &crate::app::UpsPanel, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    if theme.ascii {
        // two ascii ramps stacked: load then batt
        const RAMP: &[u8] = b" .:-=+*#%@";
        let take = area.width.saturating_sub(7) as usize;
        let ramp = |hist: &std::collections::VecDeque<u64>| -> String {
            hist.iter()
                .rev()
                .take(take)
                .rev()
                .map(|v| RAMP[((*v as usize) * (RAMP.len() - 1)) / 100] as char)
                .collect()
        };
        let mut lines = vec![Line::from(vec![
            Span::styled(" load ", Style::default().fg(theme.dim())),
            Span::raw(ramp(&panel.load_hist)),
        ])];
        if area.height >= 2 {
            lines.push(Line::from(vec![
                Span::styled(" batt ", Style::default().fg(theme.dim())),
                Span::raw(ramp(&panel.batt_hist)),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    // rich: ratatui Chart — load bars + battery line on a shared 0-100 axis
    let take = (area.width as usize).saturating_sub(2);
    let to_points = |hist: &std::collections::VecDeque<u64>| -> Vec<(f64, f64)> {
        let n = hist.len();
        let start = n.saturating_sub(take);
        hist.iter()
            .skip(start)
            .enumerate()
            .map(|(i, v)| (i as f64, *v as f64))
            .collect()
    };
    let load_pts = to_points(&panel.load_hist);
    let batt_pts = to_points(&panel.batt_hist);
    let x_max = take.max(2) as f64;

    let load_now = load_pts.last().map(|p| p.1).unwrap_or(0.0);
    let datasets = vec![
        Dataset::default()
            .name("load")
            .marker(symbols::Marker::HalfBlock)
            .graph_type(GraphType::Bar)
            .style(Style::default().fg(theme.rising(load_now)))
            .data(&load_pts),
        Dataset::default()
            .name("batt")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(theme.accent()))
            .data(&batt_pts),
    ];
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds([0.0, x_max]))
        .y_axis(Axis::default().bounds([0.0, 100.0]))
        .legend_position(None);
    frame.render_widget(chart, area);
}

fn draw_compact(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines = vec![Line::from(Span::styled(
        format!(" {:<14}{:<12}{:>6}{:>7}{:>9}", "name", "status", "load", "batt", "runtime"),
        Style::default().fg(theme.dim()),
    ))];
    for (i, p) in app.upses.iter().enumerate() {
        let status = if p.error.is_some() {
            "COMMLOST".to_string()
        } else {
            p.status.as_ref().map(|s| s.status_text().to_string()).unwrap_or_else(|| "...".into())
        };
        let (load, batt, run) = match &p.status {
            Some(s) => (fmt_opt(s.num("LOADPCT"), "%"), fmt_opt(s.num("BCHARGE"), "%"), fmt_opt(s.num("TIMELEFT"), "m")),
            None => ("--".into(), "--".into(), "--".into()),
        };
        let marker = if i == app.selected { if theme.ascii { ">" } else { "▌" } } else { " " };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme.accent())),
            Span::styled(format!("{:<14}", p.name), Style::default().add_modifier(if i == app.selected { Modifier::BOLD } else { Modifier::empty() })),
            Span::styled(format!("{status:<12}"), Style::default().fg(theme.status(&status))),
            Span::raw(format!("{load:>6}{batt:>7}{run:>9}")),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
