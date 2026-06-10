// SPDX-License-Identifier: GPL-3.0-or-later
//! Integration tests that render every view through ratatui's TestBackend
//! and assert (a) it doesn't panic, (b) basic mode is pure ASCII, (c) key
//! content is present.

use apctui::app::{App, View};
use apctui::theme::{ColorMode, Theme};
use apctui::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn render(view: View, basic: bool, w: u16, h: u16) -> String {
    let mode = if basic { ColorMode::Mono } else { ColorMode::Truecolor };
    let theme = Theme::new(mode, basic);
    let mut app = App::test_fixture(basic);
    app.test_set_view(view);
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app, &theme)).unwrap();
    // Concatenate the buffer's cell symbols into a string.
    let buf = term.backend().buffer().clone();
    let mut s = String::new();
    for cell in buf.content() {
        s.push_str(cell.symbol());
    }
    s
}

const VIEWS: &[View] = &[
    View::Dashboard,
    View::Detail,
    View::Events,
    View::Help,
];

#[test]
fn all_views_render_without_panic_rich() {
    for &v in VIEWS {
        let _ = render(v, false, 100, 30);
    }
}

#[test]
fn basic_mode_is_pure_ascii() {
    for &v in VIEWS {
        let out = render(v, true, 100, 30);
        let non_ascii: Vec<char> = out.chars().filter(|c| !c.is_ascii()).collect();
        assert!(
            non_ascii.is_empty(),
            "view {:?} leaked non-ASCII in basic mode: {:?}",
            v,
            non_ascii.iter().take(10).collect::<String>()
        );
    }
}

#[test]
fn dashboard_shows_units_and_status() {
    let out = render(View::Dashboard, false, 100, 30);
    assert!(out.contains("rack-main"));
    assert!(out.contains("rack-aux"));
    assert!(out.contains("ONLINE"));
    assert!(out.contains("ONBATT"));
}

#[test]
fn detail_shows_fields() {
    let out = render(View::Detail, false, 100, 40);
    assert!(out.contains("rack-main"));
    assert!(out.contains("Battery charge") || out.contains("Line voltage"));
}

#[test]
fn tiny_terminal_does_not_panic() {
    for &v in VIEWS {
        let _ = render(v, false, 20, 5);
        let _ = render(v, true, 20, 5);
    }
}

#[test]
fn very_narrow_but_tall() {
    let _ = render(View::Dashboard, false, 41, 50);
}

// ---- stateful views: editor, services, clientgen ----

const SAMPLE_CONF: &str =
    "UPSNAME rack-main\nUPSCABLE usb\nUPSTYPE usb\nDEVICE\nBATTERYLEVEL 10\nMINUTES 5\nTIMEOUT 0\nNETSERVER on\nNISIP 0.0.0.0\nNISPORT 3551\n";

fn render_with<F: FnOnce(&mut App)>(basic: bool, w: u16, h: u16, setup: F) -> String {
    let mode = if basic { ColorMode::Mono } else { ColorMode::Truecolor };
    let theme = Theme::new(mode, basic);
    let mut app = App::test_fixture(basic);
    setup(&mut app);
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app, &theme)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut s = String::new();
    for cell in buf.content() {
        s.push_str(cell.symbol());
    }
    s
}

fn assert_ascii(label: &str, out: &str) {
    let non: Vec<char> = out.chars().filter(|c| !c.is_ascii()).collect();
    assert!(non.is_empty(), "{label} leaked non-ASCII: {:?}", non.iter().take(12).collect::<String>());
}

#[test]
fn editor_renders_and_is_ascii_in_basic() {
    let out = render_with(false, 100, 36, |a| a.test_open_editor(SAMPLE_CONF));
    assert!(out.contains("UPSNAME"));
    assert!(out.contains("BATTERYLEVEL"));
    assert!(out.contains("Power policy")); // group header
    let basic = render_with(true, 100, 36, |a| a.test_open_editor(SAMPLE_CONF));
    assert_ascii("editor basic", &basic);
}

#[test]
fn editor_diff_overlay_ascii_in_basic() {
    let basic = render_with(true, 100, 36, |a| {
        a.test_open_editor(SAMPLE_CONF);
        a.test_editor_show_diff();
    });
    assert_ascii("editor diff basic", &basic);
    // and renders the change in rich
    let rich = render_with(false, 100, 36, |a| {
        a.test_open_editor(SAMPLE_CONF);
        a.test_editor_show_diff();
    });
    assert!(rich.contains("BATTERYLEVEL"));
}

#[test]
fn editor_invalid_value_shows_finding() {
    let bad = "UPSTYPE net\nDEVICE\n"; // net requires host:port -> error
    let out = render_with(false, 100, 36, |a| a.test_open_editor(bad));
    assert!(out.contains("error") || out.contains("DEVICE"));
}

#[test]
fn services_renders_and_is_ascii_in_basic() {
    let out = render_with(false, 100, 30, |a| a.test_open_services());
    assert!(out.contains("rack-main"));
    assert!(out.contains("active"));
    assert!(out.contains("failed"));
    let basic = render_with(true, 100, 30, |a| a.test_open_services());
    assert_ascii("services basic", &basic);
}

#[test]
fn services_confirm_modal_ascii_in_basic() {
    let basic = render_with(true, 100, 30, |a| {
        a.test_open_services();
        a.test_services_confirm_stop();
    });
    assert_ascii("services confirm basic", &basic);
    let rich = render_with(false, 100, 30, |a| {
        a.test_open_services();
        a.test_services_confirm_stop();
    });
    assert!(rich.contains("confirm"));
    assert!(rich.contains("stop"));
}

#[test]
fn clientgen_renders_and_is_ascii_in_basic() {
    let out = render_with(false, 110, 30, |a| a.test_open_clientgen());
    assert!(out.contains("UPSTYPE net"));      // preview
    assert!(out.contains("master address"));   // form
    let basic = render_with(true, 110, 30, |a| a.test_open_clientgen());
    assert_ascii("clientgen basic", &basic);
}

#[test]
fn paused_header_ascii_in_basic() {
    let basic = render_with(true, 100, 20, |a| {
        a.test_set_view(View::Dashboard);
        a.test_pause();
    });
    assert_ascii("paused basic", &basic);
}
