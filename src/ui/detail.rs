// SPDX-License-Identifier: GPL-3.0-or-later
//! Detail view: full NIS field table for the selected UPS, grouped.

use super::widgets::block;
use crate::app::App;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

// Curated field order with friendly labels; anything else is appended.
const PRIMARY: &[(&str, &str)] = &[
    ("STATUS", "Status"),
    ("LINEV", "Line voltage"),
    ("LOADPCT", "Load"),
    ("BCHARGE", "Battery charge"),
    ("TIMELEFT", "Runtime left"),
    ("BATTV", "Battery voltage"),
    ("NOMPOWER", "Nominal power"),
    ("OUTPUTV", "Output voltage"),
    ("LINEFREQ", "Line frequency"),
    ("ITEMP", "Internal temp"),
    ("NUMXFERS", "Transfers"),
    ("XOFFBATT", "Last off battery"),
    ("XONBATT", "Last on battery"),
    ("TONBATT", "Time on battery"),
    ("CUMONBATT", "Cumulative on battery"),
    ("LASTXFER", "Last transfer reason"),
    ("SELFTEST", "Self test"),
    ("MODEL", "Model"),
    ("SERIALNO", "Serial"),
    ("FIRMWARE", "Firmware"),
    ("BATTDATE", "Battery date"),
    ("MANDATE", "Mfg date"),
    ("HOSTNAME", "Daemon host"),
    ("VERSION", "apcupsd version"),
];

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(panel) = app.selected_panel() else {
        frame.render_widget(Paragraph::new(" no UPS selected"), area);
        return;
    };

    let title = Line::from(vec![
        Span::styled(format!(" {} ", panel.name), Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())),
        Span::styled(format!("{} ", panel.addr), Style::default().fg(theme.dim())),
    ]);
    let blk = block(theme).title(title).border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let mut lines = Vec::new();
    match (&panel.status, &panel.error) {
        (_, Some(err)) => {
            lines.push(Line::from(Span::styled(
                format!(" unreachable: {err}"),
                Style::default().fg(theme.error_color()),
            )));
            lines.push(Line::from(Span::styled(
                " the daemon may be stopped - check the services view (s)",
                Style::default().fg(theme.dim()),
            )));
        }
        (Some(s), None) => {
            let mut shown = std::collections::HashSet::new();
            for (key, label) in PRIMARY {
                if let Some(v) = s.get(key) {
                    shown.insert(key.to_string());
                    lines.push(field_line(theme, label, v, key));
                }
            }
            // remaining fields
            let mut extra: Vec<_> = s.fields.iter().filter(|(k, _)| !shown.contains(*k)).collect();
            extra.sort_by_key(|(k, _)| (*k).clone());
            if !extra.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" other fields", Style::default().fg(theme.dim()).add_modifier(Modifier::BOLD))));
                for (k, v) in extra {
                    lines.push(field_line(theme, k, v, k));
                }
            }
        }
        (None, None) => {
            lines.push(Line::from(Span::styled(" connecting...", Style::default().fg(theme.dim()))));
        }
    }

    let p = Paragraph::new(lines).scroll((app.detail_scroll, 0));
    frame.render_widget(p, inner);
}

fn field_line(theme: &Theme, label: &str, value: &str, key: &str) -> Line<'static> {
    let vcolor = if key == "STATUS" {
        theme.status(value)
    } else {
        theme.fg()
    };
    Line::from(vec![
        Span::styled(format!("  {label:<22}"), Style::default().fg(theme.dim())),
        Span::styled(value.to_string(), Style::default().fg(vcolor)),
    ])
}
