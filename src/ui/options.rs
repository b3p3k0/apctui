// SPDX-License-Identifier: GPL-3.0-or-later
//! App options form: notification settings, token entry, test action.

use super::widgets::block;
use crate::app::{self, App};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Mask a credential for display: first 4 chars then stars, capped.
fn mask_token(t: &str, theme: &Theme) -> String {
    if t.is_empty() {
        return theme.g_dash().to_string();
    }
    let head: String = t.chars().take(4).collect();
    let stars = t.chars().count().saturating_sub(4).min(12);
    format!("{head}{} ({} chars)", "*".repeat(stars), t.chars().count())
}

fn onoff(v: bool) -> String {
    format!("[{}]", if v { "on " } else { "off" })
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(op) = &app.options else { return };

    let [form_area, help_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);

    let blk = block(theme)
        .title(Line::from(Span::styled(
            " options ",
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()),
        )))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(form_area);
    frame.render_widget(blk, form_area);

    let w = &op.working;
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        " Notifications",
        Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD),
    )));

    for i in 0..app::OPTIONS_FIELDS {
        let selected = i == op.cursor;
        let editing = selected && op.editing;
        let marker = if selected { theme.g_rail() } else { " " };

        let value = if editing {
            format!("{}_", op.edit_buffer)
        } else {
            match i {
                0 => onoff(w.enabled),
                1 => format!("{}{}{}", theme.enum_open(), w.provider, theme.enum_close()),
                2 => mask_token(&w.pushbullet_token, theme),
                3 => onoff(w.on_battery),
                4 => onoff(w.on_line),
                5 => onoff(w.comm_lost),
                6 => onoff(w.comm_restored),
                7 => w.cooldown_secs.to_string(),
                8 => format!("{} press enter", theme.g_enter()),
                _ => String::new(),
            }
        };

        let key_style = if selected {
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())
        } else {
            Style::default()
        };
        let value_style = if editing {
            Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD)
        } else if i == 8 {
            Style::default().fg(theme.dim())
        } else {
            Style::default().fg(theme.fg())
        };

        // blank separator before the test action row
        if i == 8 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme.accent())),
            Span::styled(format!("  {:<24}", app::options_field_label(i)), key_style),
            Span::styled(value, value_style),
        ]));
    }

    // delivery state summary
    lines.push(Line::from(""));
    let state = if !w.enabled {
        Span::styled(
            format!(" {} notifications are off", theme.g_dot()),
            Style::default().fg(theme.dim()),
        )
    } else if w.pushbullet_token.is_empty() {
        Span::styled(
            format!(" {} enabled, but no token: nothing will send", theme.g_warn()),
            Style::default().fg(theme.warn_color()),
        )
    } else {
        Span::styled(
            format!(" {} active: pushes will send on save", theme.g_check()),
            Style::default().fg(theme.ok_color()),
        )
    };
    lines.push(Line::from(state));

    frame.render_widget(Paragraph::new(lines), inner);

    // contextual help for the selected field
    let hint = if op.editing {
        format!("  typing{} {} commit {} esc cancel", theme.g_ellipsis(), theme.g_enter(), theme.g_mdot())
    } else {
        format!("  {} edit/toggle {} t test {} s save", theme.g_enter(), theme.g_mdot(), theme.g_mdot())
    };
    let help_lines = vec![
        Line::from(Span::styled(
            format!("  {}", app::options_field_help(op.cursor)),
            Style::default().fg(theme.dim()),
        )),
        Line::from(Span::styled(
            hint,
            Style::default().fg(theme.dim()).add_modifier(Modifier::ITALIC),
        )),
    ];
    frame.render_widget(Paragraph::new(help_lines).wrap(Wrap { trim: true }), help_area);
}
