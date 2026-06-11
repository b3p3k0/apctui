// SPDX-License-Identifier: GPL-3.0-or-later
//! Client config generator: a small form (left) with a live conf preview
//! (right). Writes a deploy bundle on `w`.

use super::widgets::block;
use crate::app::{self, App};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

const N_FIELDS: usize = 5;

fn tab_strip(cg: &crate::app::ClientGenState, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, t) in cg.tabs.iter().enumerate() {
        let active = i == cg.active;
        let style = if active {
            Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(theme.dim())
        };
        spans.push(Span::styled(format!(" {} ", t.instance), style));
        if i + 1 < cg.tabs.len() {
            spans.push(Span::styled(
                if theme.ascii { " | " } else { " │ " },
                Style::default().fg(theme.dim()),
            ));
        }
    }
    Line::from(spans)
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(cg) = &app.clientgen else { return };
    let tab = cg.tab();

    let [form_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);

    // form
    let blk = block(theme)
        .title(tab_strip(cg, theme))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(form_area);
    frame.render_widget(blk, form_area);

    let mut lines = Vec::new();
    for i in 0..N_FIELDS {
        let selected = i == cg.cursor;
        let editing = selected && cg.editing;
        let label = app::clientgen_field_label(i);
        let value = field_value(tab, i);
        let shown = if editing { format!("{}_", cg.edit_buffer) } else { value };
        let marker = if selected { theme.g_rail() } else { " " };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme.accent())),
            Span::styled(
                format!("  {label}"),
                if selected {
                    Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())
                } else {
                    Style::default().fg(theme.dim())
                },
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                shown,
                if editing {
                    Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ),
        ]));
    }
    lines.push(Line::from(""));
    // A loopback master is unreachable from any client machine. This fires
    // when LAN detection failed at open, or the user typed loopback in.
    let master_host = tab.params.master_addr.split(':').next().unwrap_or("");
    if master_host == "localhost" || master_host.starts_with("127.") {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} master is loopback - clients can't reach it; set this host's LAN IP",
                theme.g_warn()
            ),
            Style::default().fg(theme.warn_color()).add_modifier(Modifier::BOLD),
        )));
    }

    if let Some(path) = &tab.saved_path {
        lines.push(Line::from(Span::styled(
            format!("  {} wrote {path}", theme.g_check()),
            Style::default().fg(theme.ok_color()),
        )));
        lines.push(Line::from(Span::styled(
            "  (plus an INSTALL.txt next to it)",
            Style::default().fg(theme.dim()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  w writes a deploy bundle to ~/apctui-client-bundles",
            Style::default().fg(theme.dim()),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);

    // preview
    let pblk = block(theme)
        .title(Line::from(Span::styled(" apcupsd.conf preview ", Style::default().fg(theme.dim()))))
        .border_style(Style::default().fg(theme.dim()));
    let pinner = pblk.inner(preview_area);
    frame.render_widget(pblk, preview_area);
    let preview_lines: Vec<Line> = tab.preview.lines().map(|l| {
        let style = if l.trim_start().starts_with('#') {
            Style::default().fg(theme.dim())
        } else {
            Style::default()
        };
        Line::from(Span::styled(format!(" {l}"), style))
    }).collect();
    frame.render_widget(Paragraph::new(preview_lines), pinner);
}

fn field_value(tab: &crate::app::ClientGenTab, idx: usize) -> String {
    let p = &tab.params;
    match idx {
        0 => if p.master_addr.is_empty() { "(unset, host:port)".into() } else { p.master_addr.clone() },
        1 => if p.ups_name.is_empty() { "(unset)".into() } else { p.ups_name.clone() },
        2 => format!("{}%", p.battery_level),
        3 => format!("{} min", p.minutes),
        4 => format!("{} s", p.polltime),
        _ => String::new(),
    }
}
