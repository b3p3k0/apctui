// SPDX-License-Identifier: GPL-3.0-or-later
//! Services view: list of discovered apcupsd instances with state, plus a
//! confirmation modal for state-changing actions.

use super::widgets::block;
use crate::app::App;
use crate::service::ActiveState;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(sv) = &app.services else { return };

    let blk = block(theme)
        .title(Line::from(Span::styled(" apcupsd instances ", Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()))))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let mut lines = vec![Line::from(Span::styled(
        format!("  {:<16}{:<10}{:<10}{:<22}", "instance", "active", "enabled", "nis"),
        Style::default().fg(theme.dim()),
    ))];
    for (i, inst) in sv.instances.iter().enumerate() {
        let selected = i == sv.cursor;
        let marker = if selected { theme.g_rail() } else { " " };
        let (acolor, atext) = match inst.active {
            ActiveState::Active => (theme.ok_color(), "active"),
            ActiveState::Failed => (theme.error_color(), "failed"),
            ActiveState::Inactive => (theme.dim(), "inactive"),
            ActiveState::Unknown => (theme.dim(), "unknown"),
        };
        let en = if inst.enabled { "enabled" } else { "disabled" };
        let stock = inst.name == "apcupsd";
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme.accent())),
            Span::styled(
                format!("  {:<16}", inst.name),
                if selected { Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()) }
                else if stock { Style::default().fg(theme.dim()) }
                else { Style::default() },
            ),
            Span::styled(format!("{atext:<10}"), Style::default().fg(acolor)),
            Span::styled(format!("{en:<10}"), Style::default().fg(theme.dim())),
            Span::styled(
                inst.nis_addr.clone().unwrap_or_else(|| theme.g_none().to_string()),
                Style::default().fg(theme.dim()),
            ),
        ]));
    }
    if sv.instances.iter().any(|i| i.name == "apcupsd") {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  note: 'apcupsd' is the stock single-instance unit (read-only here)",
            Style::default().fg(theme.dim()).add_modifier(Modifier::ITALIC),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if let Some((action, name)) = &sv.confirm {
        draw_confirm(frame, area, action.verb(), name, theme);
    }
}

fn draw_confirm(frame: &mut Frame, area: Rect, verb: &str, name: &str, theme: &Theme) {
    let w = 54.min(area.width);
    let h = 7.min(area.height);
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    let warn = matches!(verb, "stop" | "disable");
    let bcolor = if warn { theme.warn_color() } else { theme.accent() };
    let blk = block(theme).border_style(Style::default().fg(bcolor))
        .title(Span::styled(" confirm ", Style::default().add_modifier(Modifier::BOLD).fg(bcolor)));
    let inner = blk.inner(popup);
    frame.render_widget(blk, popup);

    let extra = if verb == "stop" {
        "  this UPS will stop being monitored; shutdown protection ends."
    } else if verb == "disable" {
        "  it will not start on the next boot."
    } else {
        ""
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {verb} apcupsd@{name}?"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(extra, Style::default().fg(theme.warn_color()))),
        Line::from(""),
        Line::from(Span::styled("  y confirm    n cancel", Style::default().fg(theme.dim()))),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
    let _ = Color::Reset;
}
