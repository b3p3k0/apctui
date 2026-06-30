// SPDX-License-Identifier: GPL-3.0-or-later
//! Units view: list monitored + configured UPS units and add/remove LAN
//! (remote) endpoints. Edits persist to config.toml and take effect on the
//! next launch — there is no live poller spawn.

use super::widgets::block;
use crate::app::{AddField, AddForm, App, UnitKind};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(u) = &app.units else { return };

    let blk = block(theme)
        .title(Line::from(Span::styled(
            " Units ",
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()),
        )))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let mut lines = vec![Line::from(Span::styled(
        format!("  {:<16}{:<24}{}", "name", "address", "source"),
        Style::default().fg(theme.dim()),
    ))];
    for (i, row) in u.rows.iter().enumerate() {
        let selected = i == u.cursor;
        let marker = if selected { theme.g_rail() } else { " " };
        let (stag, scolor) = match row.kind {
            UnitKind::Local => ("local (auto)", theme.dim()),
            UnitKind::Config => ("LAN (config)", theme.fg()),
        };
        let name_style = if selected {
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(theme.accent())),
            Span::styled(format!("  {:<16}", row.name), name_style),
            Span::styled(format!("{:<24}", row.addr), Style::default().fg(theme.dim())),
            Span::styled(stag, Style::default().fg(scolor)),
        ];
        if row.pending {
            spans.push(Span::styled(
                format!("   {} pending", theme.g_dot()),
                Style::default().fg(theme.warn_color()),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  changes apply after restart",
        Style::default().fg(theme.dim()).add_modifier(Modifier::ITALIC),
    )));
    frame.render_widget(Paragraph::new(lines), inner);

    if let Some(form) = &u.form {
        draw_add_form(frame, area, form, theme);
    } else if let Some(name) = &u.confirm_remove {
        let addr = u
            .rows
            .iter()
            .find(|r| &r.name == name)
            .map(|r| r.addr.clone())
            .unwrap_or_default();
        draw_remove_confirm(frame, area, name, &addr, theme);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}

fn draw_add_form(frame: &mut Frame, area: Rect, form: &AddForm, theme: &Theme) {
    let popup = centered(area, 54, 9);
    frame.render_widget(Clear, popup);
    let blk = block(theme)
        .border_style(Style::default().fg(theme.accent()))
        .title(Span::styled(
            " Add LAN UPS ",
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()),
        ));
    let inner = blk.inner(popup);
    frame.render_widget(blk, popup);

    let field = |label: &str, val: &str, active: bool| -> Line {
        let shown = if active { format!("{val}_") } else { val.to_string() };
        let vstyle = if active {
            Style::default().fg(theme.accent())
        } else {
            Style::default()
        };
        Line::from(vec![
            Span::styled(format!("  {label:<5} ["), Style::default().fg(theme.dim())),
            Span::styled(format!(" {shown} "), vstyle),
            Span::styled("]", Style::default().fg(theme.dim())),
        ])
    };
    let lines = vec![
        Line::from(""),
        field("name", &form.name, form.field == AddField::Name),
        field("host", &form.host, form.field == AddField::Host),
        Line::from(Span::styled(
            "        port defaults to :3551 if omitted",
            Style::default().fg(theme.dim()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {} save   tab next field   esc cancel", theme.g_enter()),
            Style::default().fg(theme.dim()),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_remove_confirm(frame: &mut Frame, area: Rect, name: &str, addr: &str, theme: &Theme) {
    let popup = centered(area, 52, 7);
    frame.render_widget(Clear, popup);
    let blk = block(theme)
        .border_style(Style::default().fg(theme.warn_color()))
        .title(Span::styled(
            " Remove unit ",
            Style::default().add_modifier(Modifier::BOLD).fg(theme.warn_color()),
        ));
    let inner = blk.inner(popup);
    frame.render_widget(blk, popup);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Remove \"{name}\" ({addr})?"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Takes effect after restart.",
            Style::default().fg(theme.dim()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  y remove    n cancel",
            Style::default().fg(theme.dim()),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
