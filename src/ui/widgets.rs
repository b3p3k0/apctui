// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared UI building blocks: themed borders, labeled bars, toast colors.

use crate::theme::Theme;
use ratatui::style::{Color, Style};
use ratatui::symbols::border;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};

const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// A themed bordered block (rounded in rich mode, +/-/| in ascii mode).
pub fn block(theme: &Theme) -> Block<'static> {
    let b = Block::default().borders(Borders::ALL);
    if theme.ascii {
        b.border_set(ASCII_BORDER)
    } else {
        b.border_type(ratatui::widgets::BorderType::Rounded)
    }
}

/// Build a labeled bar span set: "label NN% [bar]" with the bar colored by
/// the supplied color. Returns spans so callers can place them in a Line.
pub fn labeled_bar<'a>(
    theme: &Theme,
    label: &'a str,
    pct: Option<f64>,
    width: usize,
    color: Color,
) -> Vec<Span<'a>> {
    let frac = pct.unwrap_or(0.0) / 100.0;
    let bar = theme.bar(frac, width);
    let value = match pct {
        Some(p) => format!("{p:>3.0}%"),
        None => " --%".to_string(),
    };
    vec![
        Span::styled(format!("{label} "), Style::default().fg(theme.dim())),
        Span::styled(value, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(color)),
    ]
}

pub struct ToastColors {
    pub fg: Color,
    pub prefix: &'static str,
}

impl ToastColors {
    pub fn for_kind(kind: crate::app::ToastKind, theme: &Theme) -> Self {
        use crate::app::ToastKind::*;
        match kind {
            Info => ToastColors { fg: theme.accent(), prefix: if theme.ascii { "i" } else { "ℹ" } },
            Success => ToastColors { fg: theme.ok_color(), prefix: if theme.ascii { "ok" } else { "✓" } },
            Error => ToastColors { fg: theme.error_color(), prefix: if theme.ascii { "!" } else { "✗" } },
        }
    }
}
