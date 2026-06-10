// SPDX-License-Identifier: GPL-3.0-or-later
//! Centralized config editor: one tab per instance, grouped directive list
//! with inline editing, live validation banner, diff-before-save overlay.

use super::widgets::block;
use crate::app::{App, EditorState, EditorTab};
use crate::config::{self, Kind, Severity};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

fn ed_hint_editing(theme: &Theme) -> String {
    format!("  typing{} {} commit {} esc cancel", theme.g_ellipsis(), theme.g_enter(), theme.g_mdot())
}
fn ed_hint_idle(theme: &Theme) -> String {
    format!("  {} edit {} space toggle/cycle {} tab switch unit", theme.g_enter(), theme.g_mdot(), theme.g_mdot())
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(ed) = &app.editor else { return };
    let tab = ed.tab();

    let n_findings = tab.findings.len();
    let banner_h: u16 = if n_findings == 0 { 1 } else { (n_findings as u16).min(4) + 1 };

    let [list_area, help_area, banner_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(banner_h),
    ])
    .areas(area);

    draw_list(frame, list_area, ed, theme);
    draw_field_help(frame, help_area, ed, theme);
    draw_banner(frame, banner_area, tab, theme);

    if ed.show_diff {
        draw_diff_overlay(frame, area, tab, theme);
    }
}

/// The tab strip: one entry per instance, active highlighted, dirty marked.
fn tab_strip(ed: &EditorState, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, t) in ed.tabs.iter().enumerate() {
        let active = i == ed.active;
        let mut label = format!(" {} ", t.instance);
        if t.dirty() {
            label = format!(" {} {}", t.instance, theme.g_dot());
        }
        let style = if active {
            Style::default()
                .fg(theme.accent())
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if t.dirty() {
            Style::default().fg(theme.warn_color())
        } else {
            Style::default().fg(theme.dim())
        };
        spans.push(Span::styled(label, style));
        if i + 1 < ed.tabs.len() {
            spans.push(Span::styled(
                if theme.ascii { " | " } else { " │ " },
                Style::default().fg(theme.dim()),
            ));
        }
    }
    Line::from(spans)
}

fn draw_list(frame: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    let tab = ed.tab();
    let blk = block(theme)
        .title(tab_strip(ed, theme))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let mut lines = Vec::new();
    let mut last_group: Option<config::Group> = None;
    let rows = inner.height as usize;
    let mut line_index_of_cursor = 0usize;
    {
        let mut idx = 0;
        let mut lg: Option<config::Group> = None;
        for (i, f) in tab.fields.iter().enumerate() {
            if lg != Some(f.group) {
                idx += 1;
                lg = Some(f.group);
            }
            if i == tab.cursor {
                line_index_of_cursor = idx;
            }
            idx += 1;
        }
    }
    let offset = line_index_of_cursor.saturating_sub(rows.saturating_sub(2));

    for (i, f) in tab.fields.iter().enumerate() {
        if last_group != Some(f.group) {
            lines.push(Line::from(Span::styled(
                format!(" {}", f.group.title()),
                Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD),
            )));
            last_group = Some(f.group);
        }
        let selected = i == tab.cursor;
        lines.push(field_row(ed, tab, f, selected, theme));
    }

    let p = Paragraph::new(lines).scroll((offset as u16, 0));
    frame.render_widget(p, inner);
}

fn field_row(
    ed: &EditorState,
    tab: &EditorTab,
    f: &crate::app::EditField,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { theme.g_rail() } else { " " };
    let editing = selected && ed.editing;

    let value_str = if editing {
        format!("{}_", ed.edit_buffer)
    } else {
        match &f.kind {
            Kind::Bool => {
                if f.value.is_empty() {
                    theme.g_dash().to_string()
                } else {
                    format!("[{}]", if f.value.eq_ignore_ascii_case("on") { "on " } else { "off" })
                }
            }
            Kind::Enum(_) => format!(
                "{}{}{}",
                theme.enum_open(),
                if f.value.is_empty() { theme.g_dash() } else { &f.value },
                theme.enum_close()
            ),
            _ => if f.value.is_empty() { theme.g_dash().to_string() } else { f.value.clone() },
        }
    };

    let finding = tab.findings.iter().find(|x| x.key.as_deref() == Some(f.key.as_str()));
    let flag = match finding.map(|x| &x.severity) {
        Some(Severity::Error) => Span::styled(format!(" {}", theme.g_cross()), Style::default().fg(theme.error_color())),
        Some(Severity::Warning) => Span::styled(format!(" {}", theme.g_warn()), Style::default().fg(theme.warn_color())),
        None => Span::raw(""),
    };

    let key_style = if selected {
        Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())
    } else if f.present {
        Style::default()
    } else {
        Style::default().fg(theme.dim())
    };
    let value_style = if editing {
        Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(theme.accent())),
        Span::styled(format!("  {:<16}", f.key), key_style),
        Span::styled(value_str, value_style),
        flag,
    ])
}

fn draw_field_help(frame: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    let tab = ed.tab();
    let help = tab.fields.get(tab.cursor).map(|f| f.help.as_str()).unwrap_or("");
    let hint = if ed.editing { ed_hint_editing(theme) } else { ed_hint_idle(theme) };
    let lines = vec![
        Line::from(Span::styled(format!("  {help}"), Style::default().fg(theme.dim()))),
        Line::from(Span::styled(hint, Style::default().fg(theme.dim()).add_modifier(Modifier::ITALIC))),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn draw_banner(frame: &mut Frame, area: Rect, tab: &EditorTab, theme: &Theme) {
    if tab.findings.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {} no issues", theme.g_check()),
                Style::default().fg(theme.ok_color()),
            )),
            area,
        );
        return;
    }
    let errors = tab.findings.iter().filter(|f| f.severity == Severity::Error).count();
    let warns = tab.findings.len() - errors;
    let mut lines = vec![Line::from(Span::styled(
        format!(" {errors} error(s), {warns} warning(s):"),
        Style::default()
            .fg(if errors > 0 { theme.error_color() } else { theme.warn_color() })
            .add_modifier(Modifier::BOLD),
    ))];
    for f in tab.findings.iter().take(3) {
        let (glyph, color) = match f.severity {
            Severity::Error => (theme.g_cross(), theme.error_color()),
            Severity::Warning => (theme.g_warn(), theme.warn_color()),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {glyph} "), Style::default().fg(color)),
            Span::styled(f.message.clone(), Style::default().fg(color)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn draw_diff_overlay(frame: &mut Frame, area: Rect, tab: &EditorTab, theme: &Theme) {
    let w = (area.width as f32 * 0.9) as u16;
    let h = (area.height as f32 * 0.9) as u16;
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);

    let title = Line::from(vec![
        Span::styled(
            format!(" review changes: {} ", tab.instance),
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()),
        ),
        Span::styled(format!("s save {} esc back ", theme.g_mdot()), Style::default().fg(theme.dim())),
    ]);
    let blk = block(theme).title(title).border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(popup);
    frame.render_widget(blk, popup);

    let diff = config::diff::diff(&tab.original.serialize(), &tab.working.serialize());
    let compact = config::diff::compact(&diff, 2);
    let lines: Vec<Line> = compact
        .iter()
        .map(|d| match d {
            config::diff::DiffLine::Context(s) => {
                let shown = if s == "…" { theme.g_ellipsis() } else { s.as_str() };
                Line::from(Span::styled(format!("  {shown}"), Style::default().fg(theme.dim())))
            }
            config::diff::DiffLine::Removed(s) => {
                Line::from(Span::styled(format!("- {s}"), Style::default().fg(theme.error_color())))
            }
            config::diff::DiffLine::Added(s) => {
                Line::from(Span::styled(format!("+ {s}"), Style::default().fg(theme.ok_color())))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
