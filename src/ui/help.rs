// SPDX-License-Identifier: GPL-3.0-or-later
//! Help overlay popup.

use super::widgets::block;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme) {
    let w = 56.min(area.width);
    let h = 20.min(area.height);
    if area.width < 20 || area.height < 8 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    let blk = block(theme)
        .title(Span::styled(" apctui help ", Style::default().add_modifier(Modifier::BOLD).fg(theme.accent())))
        .border_style(Style::default().fg(theme.accent()));
    let inner = blk.inner(popup);
    frame.render_widget(blk, popup);

    let rows = [
        ("dashboard", ""),
        ("  enter/l", "open detail view for the selected UPS"),
        ("  j / k", "move selection"),
        ("  c", "edit the selected UPS's config"),
        ("  s", "service control (start/stop/restart)"),
        ("  g", "generate a network-client config"),
        ("  o", "options (notifications)"),
        ("  e", "view event log"),
        ("  b", "toggle basic (ASCII, no color) mode"),
        ("  p", "pause/resume sampling"),
        ("editor", ""),
        ("  enter", "edit text/number field"),
        ("  space", "toggle on/off or cycle enum"),
        ("  d", "review diff before saving"),
        ("  s", "save (escalates via pkexec/sudo) + restart"),
        ("global", ""),
        ("  q / esc", "back / quit"),
    ];
    let lines: Vec<Line> = rows.iter().map(|(k, v)| {
        if v.is_empty() {
            Line::from(Span::styled(format!(" {k}"), Style::default().fg(theme.accent()).add_modifier(Modifier::BOLD)))
        } else {
            Line::from(vec![
                Span::styled(format!(" {k:<10}"), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {v}"), Style::default().fg(theme.dim())),
            ])
        }
    }).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
