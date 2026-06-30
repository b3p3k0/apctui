// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering. View router + shared chrome (header, footer, toast). Each view
//! has its own draw fn. Two visual tiers via `Theme`: rich (rounded borders,
//! gradient block bars, sparklines) and basic/ascii (plain borders, [###]
//! bars). All glyphs in basic mode are pure ASCII.

mod widgets;

use crate::app::{App, View};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use widgets::ToastColors;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    if area.width < 40 || area.height < 8 {
        frame.render_widget(Paragraph::new("terminal too small (min 40x8)"), area);
        return;
    }

    let has_toast = app.toast.is_some();
    let constraints = if has_toast {
        vec![Constraint::Length(1), Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]
    };
    let chunks = Layout::vertical(constraints).split(area);
    let header = chunks[0];
    let body = chunks[1];

    draw_header(frame, header, app, theme);

    match app.view {
        View::Dashboard => dashboard::draw(frame, body, app, theme),
        View::Detail => detail::draw(frame, body, app, theme),
        View::Editor => editor::draw(frame, body, app, theme),
        View::Services => services::draw(frame, body, app, theme),
        View::ClientGen => clientgen::draw(frame, body, app, theme),
        View::Events => events::draw(frame, body, app, theme),
        View::Options => options::draw(frame, body, app, theme),
        View::Units => units::draw(frame, body, app, theme),
        View::Help => {
            dashboard::draw(frame, body, app, theme);
            help::draw(frame, body, theme);
        }
    }

    if has_toast {
        draw_toast(frame, chunks[2], app, theme);
        draw_footer(frame, chunks[3], app, theme);
    } else {
        draw_footer(frame, chunks[2], app, theme);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let title = match app.view {
        View::Dashboard => "dashboard",
        View::Detail => "detail",
        View::Editor => "config editor",
        View::Services => "services",
        View::ClientGen => "client config generator",
        View::Events => "events",
        View::Options => "options",
        View::Units => "units",
        View::Help => "help",
    };
    let mut spans = vec![
        Span::styled(" apctui ", Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD)),
        Span::styled(if theme.ascii { "| " } else { "│ " }, Style::default().fg(theme.dim())),
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
    ];
    if app.view == View::Dashboard {
        spans.push(Span::styled(
            format!("  {} unit{}", app.upses.len(), if app.upses.len() == 1 { "" } else { "s" }),
            Style::default().fg(theme.dim()),
        ));
        // Armed-state at a glance. Standby means another running instance
        // holds this machine's notification duty (no duplicate pushes).
        match app.notifier_state() {
            crate::notify::NotifierState::Active => spans.push(Span::styled(
                format!("  {} notify on", theme.g_dot()),
                Style::default().fg(theme.ok_color()),
            )),
            crate::notify::NotifierState::Standby => spans.push(Span::styled(
                format!("  {} notify standby", theme.g_dot()),
                Style::default().fg(theme.dim()),
            )),
            crate::notify::NotifierState::Disabled => {}
        }
    }
    if app.paused {
        let pz = if theme.ascii { "  [PAUSED]" } else { "  ⏸ PAUSED" };
        spans.push(Span::styled(pz, Style::default().fg(theme.warn_color())));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let keys = if theme.ascii {
        match app.view {
            View::Dashboard => "enter detail  c config  o options  u units  s services  g client-gen  e events  b basic  ? help  q quit",
            View::Detail => "up/dn scroll  c config  esc back  q quit",
            View::Editor => "tab unit  up/dn field  enter edit  space toggle  d diff  s save  esc close",
            View::Services => "up/dn select  r restart  S start  x stop  e enable  d disable  R rescan  esc back",
            View::ClientGen => "tab unit  up/dn field  enter edit  w write bundle  esc back",
            View::Events => "up/dn scroll  r reload  esc back",
            View::Options => "up/dn field  enter edit/toggle  t test  s save  esc back",
            View::Units => "up/dn select  a add  x remove  esc back",
            View::Help => "esc close",
        }
    } else {
        match app.view {
            View::Dashboard => "↵ detail  c config  o options  u units  s services  g client-gen  e events  b basic  ? help  q quit",
            View::Detail => "↑↓ scroll  c config  esc back  q quit",
            View::Editor => "⇥ unit  ↑↓ field  ↵ edit  space toggle  d diff  s save  esc close",
            View::Services => "↑↓ select  r restart  S start  x stop  e enable  d disable  R rescan  esc back",
            View::ClientGen => "⇥ unit  ↑↓ field  ↵ edit  w write bundle  esc back",
            View::Events => "↑↓ scroll  r reload  esc back",
            View::Options => "↑↓ field  ↵ edit/toggle  t test  s save  esc back",
            View::Units => "↑↓ select  a add  x remove  esc back",
            View::Help => "esc close",
        }
    };
    let mode = format!("{:?}{} ", theme.mode, if theme.ascii { "+ascii" } else { "" });
    let avail = area.width as usize;
    let keys_trunc = if keys.chars().count() + mode.len() + 1 > avail {
        let take = avail.saturating_sub(mode.len() + 2);
        keys.chars().take(take).collect::<String>()
    } else {
        keys.to_string()
    };
    let pad = avail.saturating_sub(keys_trunc.chars().count() + mode.len());
    let line = Line::from(vec![
        Span::styled(format!(" {keys_trunc}"), Style::default().fg(theme.dim())),
        Span::raw(" ".repeat(pad.saturating_sub(1))),
        Span::styled(
            mode,
            Style::default().fg(theme.accent()).add_modifier(Modifier::ITALIC),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_toast(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(toast) = &app.toast else { return };
    let ToastColors { fg, prefix } = ToastColors::for_kind(toast.kind, theme);
    let line = Line::from(vec![
        Span::styled(format!(" {prefix} "), Style::default().fg(fg).add_modifier(Modifier::BOLD)),
        Span::styled(&toast.text, Style::default().fg(fg)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

mod dashboard;
mod detail;
mod editor;
mod services;
mod clientgen;
mod events;
mod options;
mod units;
mod help;
