// SPDX-License-Identifier: GPL-3.0-or-later
//! Events view: tail of the apcupsd event logs.

use super::widgets::block;
use crate::app::App;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let blk = block(theme)
        .title(Line::from(Span::styled(
            " events (newest first) ",
            Style::default().add_modifier(Modifier::BOLD).fg(theme.accent()),
        )))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let lines: Vec<Line> = app.events.iter().map(|l| {
        let color = if l.contains("ONBATT") || l.contains("Power failure") {
            theme.warn_color()
        } else if l.contains("LOWBATT") || l.contains("COMMLOST") || l.contains("Communications") {
            theme.error_color()
        } else if l.contains("ONLINE") || l.contains("Power is back") {
            theme.ok_color()
        } else {
            theme.fg()
        };
        Line::from(Span::styled(format!(" {l}"), Style::default().fg(color)))
    }).collect();

    frame.render_widget(
        Paragraph::new(lines).scroll((app.events_scroll, 0)).wrap(Wrap { trim: false }),
        inner,
    );
}
