// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal capability detection and the color theme.
//!
//! Tiers: Truecolor (smooth gradients) -> Indexed256 -> Basic16 -> Mono.
//! `--basic` forces Mono + ASCII borders; NO_COLOR forces Mono but keeps
//! unicode glyphs (per https://no-color.org semantics: colors off, not style).

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMode {
    Truecolor,
    Indexed256,
    Basic16,
    Mono,
}

pub fn detect() -> ColorMode {
    if std::env::var_os("NO_COLOR").is_some() {
        return ColorMode::Mono;
    }
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return ColorMode::Truecolor;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        return ColorMode::Mono;
    }
    if term.contains("256") {
        return ColorMode::Indexed256;
    }
    ColorMode::Basic16
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub mode: ColorMode,
    /// ASCII-only glyphs and plain borders (the "basic monitor" mode).
    pub ascii: bool,
}

impl Theme {
    pub fn new(mode: ColorMode, ascii: bool) -> Self {
        Self { mode, ascii }
    }

    fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let c = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
        Color::Rgb(c(a.0, b.0), c(a.1, b.1), c(a.2, b.2))
    }

    /// Green -> yellow -> red as `pct` rises (load, temperature, ...).
    pub fn rising(&self, pct: f64) -> Color {
        match self.mode {
            ColorMode::Truecolor => {
                if pct < 50.0 {
                    Self::lerp((80, 250, 123), (241, 250, 140), pct / 50.0)
                } else {
                    Self::lerp((241, 250, 140), (255, 85, 85), (pct - 50.0) / 50.0)
                }
            }
            ColorMode::Indexed256 => match pct as u32 {
                0..=49 => Color::Indexed(84),
                50..=79 => Color::Indexed(220),
                _ => Color::Indexed(196),
            },
            ColorMode::Basic16 => match pct as u32 {
                0..=49 => Color::Green,
                50..=79 => Color::Yellow,
                _ => Color::Red,
            },
            ColorMode::Mono => Color::Reset,
        }
    }

    /// Red -> yellow -> green as `pct` rises (battery charge: high is good).
    pub fn falling(&self, pct: f64) -> Color {
        self.rising(100.0 - pct)
    }

    pub fn status(&self, status: &str) -> Color {
        if self.mode == ColorMode::Mono {
            return Color::Reset;
        }
        let s = status.to_ascii_uppercase();
        if s.contains("COMMLOST") || s.contains("LOWBATT") || s.contains("SHUTTING") {
            Color::Red
        } else if s.contains("ONBATT") || s.contains("OVERLOAD") || s.contains("REPLACEBATT") {
            Color::Yellow
        } else if s.contains("ONLINE") {
            match self.mode {
                ColorMode::Truecolor => Color::Rgb(80, 250, 123),
                _ => Color::Green,
            }
        } else {
            Color::DarkGray
        }
    }

    pub fn accent(&self) -> Color {
        match self.mode {
            ColorMode::Truecolor => Color::Rgb(139, 233, 253),
            ColorMode::Indexed256 => Color::Indexed(117),
            ColorMode::Basic16 => Color::Cyan,
            ColorMode::Mono => Color::Reset,
        }
    }

    pub fn dim(&self) -> Color {
        match self.mode {
            ColorMode::Mono => Color::Reset,
            _ => Color::DarkGray,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_endpoints() {
        let t = Theme::new(ColorMode::Truecolor, false);
        assert_eq!(t.rising(0.0), Color::Rgb(80, 250, 123));
        assert_eq!(t.rising(100.0), Color::Rgb(255, 85, 85));
    }

    #[test]
    fn mono_is_colorless() {
        let t = Theme::new(ColorMode::Mono, true);
        assert_eq!(t.rising(90.0), Color::Reset);
        assert_eq!(t.status("ONBATT"), Color::Reset);
    }
}

// ---- rendering helpers added for the full UI ----

impl Theme {
    /// A smooth horizontal bar built from Unicode eighth-blocks (rich) or
    /// ASCII (basic). `frac` in 0..=1, `width` in cells. Returns the string
    /// plus the color to render it.
    pub fn bar(&self, frac: f64, width: usize) -> String {
        let frac = frac.clamp(0.0, 1.0);
        if self.ascii {
            let filled = (frac * width as f64).round() as usize;
            format!("[{}{}]", "#".repeat(filled), ".".repeat(width.saturating_sub(filled)))
        } else {
            // eighth-block precision
            const BLOCKS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
            let total_eighths = (frac * width as f64 * 8.0).round() as usize;
            let full = total_eighths / 8;
            let rem = total_eighths % 8;
            let mut s = String::new();
            for _ in 0..full.min(width) {
                s.push('█');
            }
            if full < width {
                s.push(BLOCKS[rem]);
                for _ in (full + 1)..width {
                    s.push(' ');
                }
            }
            s
        }
    }

    /// Power-flow glyph reflecting line/battery state.
    pub fn flow_glyph(&self, on_battery: bool, comm_lost: bool) -> &'static str {
        if comm_lost {
            return if self.ascii { "??" } else { "⚠ " };
        }
        if on_battery {
            if self.ascii { "BATT" } else { "🗲 " }
        } else if self.ascii {
            "MAIN"
        } else {
            "⚡"
        }
    }

}

impl Theme {
    pub fn warn_color(&self) -> ratatui::style::Color {
        match self.mode {
            ColorMode::Mono => ratatui::style::Color::Reset,
            _ => ratatui::style::Color::Yellow,
        }
    }
    pub fn error_color(&self) -> ratatui::style::Color {
        match self.mode {
            ColorMode::Mono => ratatui::style::Color::Reset,
            _ => ratatui::style::Color::Red,
        }
    }
    pub fn ok_color(&self) -> ratatui::style::Color {
        match self.mode {
            ColorMode::Truecolor => ratatui::style::Color::Rgb(80, 250, 123),
            ColorMode::Mono => ratatui::style::Color::Reset,
            _ => ratatui::style::Color::Green,
        }
    }
    pub fn fg(&self) -> ratatui::style::Color {
        ratatui::style::Color::Reset
    }
}

// ---- glyph helpers: ASCII-safe in basic mode ----
impl Theme {
    pub fn g_rail(&self) -> &'static str { if self.ascii { ">" } else { "▌" } }
    pub fn g_check(&self) -> &'static str { if self.ascii { "ok" } else { "✓" } }
    pub fn g_cross(&self) -> &'static str { if self.ascii { "x" } else { "✗" } }
    pub fn g_warn(&self) -> &'static str { if self.ascii { "!" } else { "⚠" } }
    pub fn g_dash(&self) -> &'static str { if self.ascii { "-" } else { "—" } }
    pub fn g_dot(&self) -> &'static str { if self.ascii { "*" } else { "●" } }
    pub fn g_ellipsis(&self) -> &'static str { if self.ascii { "..." } else { "…" } }
    pub fn g_enter(&self) -> &'static str { if self.ascii { "enter" } else { "↵" } }
    pub fn g_mdot(&self) -> &'static str { if self.ascii { "-" } else { "·" } }
    pub fn g_none(&self) -> &'static str { if self.ascii { "-" } else { "—" } }
    pub fn enum_open(&self) -> &'static str { if self.ascii { "<" } else { "‹" } }
    pub fn enum_close(&self) -> &'static str { if self.ascii { ">" } else { "›" } }
}
