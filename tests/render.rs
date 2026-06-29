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
    // Two units, too narrow for side-by-side: must fall back to the stack and
    // still render both, no panic.
    let out = render_grid(false, 41, 50, 2);
    assert!(out.contains("rack-1"));
    assert!(out.contains("rack-2"));
}

// ---- grid layout (1-4 cards) + 5+ compact fallback ----

/// Render the dashboard with `n` synthetic ONLINE units named rack-1..rack-n.
fn render_grid(basic: bool, w: u16, h: u16, n: usize) -> String {
    use apctui::nis::UpsStatus;
    use apctui::registry::UpsRef;
    let mode = if basic { ColorMode::Mono } else { ColorMode::Truecolor };
    let theme = Theme::new(mode, basic);
    let refs: Vec<UpsRef> = (1..=n)
        .map(|i| UpsRef { name: format!("rack-{i}"), addr: format!("127.0.0.1:{}", 3550 + i) })
        .collect();
    let mut app = App::new(&refs, basic, apctui::options::Notifications::default());
    for panel in app.upses.iter_mut() {
        let mut fields = std::collections::HashMap::new();
        for (k, v) in [
            ("STATUS", "ONLINE"), ("LINEV", "121.5 Volts"), ("LOADPCT", "42.0 Percent"),
            ("BCHARGE", "93.0 Percent"), ("TIMELEFT", "22.0 Minutes"), ("BATTV", "27.3 Volts"),
            ("NOMPOWER", "900 Watts"), ("MODEL", "Smart-UPS 1500"), ("NUMXFERS", "3"),
        ] {
            fields.insert(k.to_string(), v.to_string());
        }
        panel.status = Some(UpsStatus { fields });
    }
    app.test_set_view(View::Dashboard);
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

/// y of the first row in a w×h buffer string that contains `needle`. The
/// buffer string is one char per cell in row-major order, so chunk by chars
/// (box-drawing symbols are multi-byte, so byte-chunking would misalign).
fn row_of(out: &str, w: u16, needle: &str) -> Option<usize> {
    let chars: Vec<char> = out.chars().collect();
    chars
        .chunks(w as usize)
        .position(|row| row.iter().collect::<String>().contains(needle))
}

#[test]
fn two_units_render_side_by_side() {
    let w = 100u16;
    let out = render_grid(false, w, 30, 2);
    assert!(out.contains("rack-1"));
    assert!(out.contains("rack-2"));
    // Both card titles sit on the same top-border row -> two columns, not rows.
    assert_eq!(row_of(&out, w, "rack-1"), row_of(&out, w, "rack-2"));
    assert_ascii("grid 2-up", &render_grid(true, w, 30, 2));
}

#[test]
fn three_units_show_no_device_placeholder() {
    let out = render_grid(false, 100, 30, 3);
    for i in 1..=3 {
        assert!(out.contains(&format!("rack-{i}")));
    }
    assert!(out.contains("no device"));
    assert_ascii("grid 3-up", &render_grid(true, 100, 30, 3));
}

#[test]
fn four_units_fill_quad() {
    let out = render_grid(false, 100, 30, 4);
    for i in 1..=4 {
        assert!(out.contains(&format!("rack-{i}")));
    }
    assert!(!out.contains("no device"));
    assert_ascii("grid 4-up", &render_grid(true, 100, 30, 4));
}

#[test]
fn five_units_fall_back_to_compact() {
    let out = render_grid(false, 100, 40, 5);
    // Compact table header is present; grid declined.
    assert!(out.contains("name"));
    assert!(out.contains("runtime"));
    assert!(!out.contains("no device"));
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

#[test]
fn history_chart_is_right_anchored() {
    // A single unit fills the dashboard width, so the chart window is ~96 cols.
    // Its 50 samples of history are narrower than that, so a partially-filled
    // chart must hug the RIGHT edge — btop convention — leaving the left empty.
    // (Single unit on purpose: the grid layout makes a 2-up card half-width,
    // which would move these coordinates; the anchor logic is the same.)
    use apctui::nis::UpsStatus;
    use apctui::registry::UpsRef;
    let theme = Theme::new(ColorMode::Truecolor, false);
    let refs = [UpsRef { name: "rack-main".into(), addr: "127.0.0.1:3551".into() }];
    let mut app = App::new(&refs, false, apctui::options::Notifications::default());
    let mut fields = std::collections::HashMap::new();
    for (k, v) in [
        ("STATUS", "ONLINE"), ("LINEV", "121.5 Volts"), ("LOADPCT", "42.0 Percent"),
        ("BCHARGE", "93.0 Percent"), ("TIMELEFT", "22.0 Minutes"), ("BATTV", "27.3 Volts"),
        ("NOMPOWER", "900 Watts"), ("MODEL", "Smart-UPS 1500"), ("NUMXFERS", "3"),
    ] {
        fields.insert(k.to_string(), v.to_string());
    }
    app.upses[0].status = Some(UpsStatus { fields });
    for i in 0..50u64 {
        app.upses[0].load_hist.push_back(30 + i % 20);
        app.upses[0].batt_hist.push_back(90 + i % 10);
    }
    app.test_set_view(View::Dashboard);
    let backend = TestBackend::new(100, 26);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app, &theme)).unwrap();
    let buf = term.backend().buffer().clone();

    let is_plot_glyph = |s: &str| {
        s.chars().next().map(|c| {
            ('\u{2580}'..='\u{259F}').contains(&c)   // block elements (bars)
                || ('\u{2800}'..='\u{28FF}').contains(&c) // braille (batt line)
        }).unwrap_or(false)
    };

    // First card chart rows (bars row=2, stats=3, chart 4..=10 inside card).
    let mut left_hits = 0;
    let mut right_hits = 0;
    for y in 4..=10u16 {
        for x in 3..=30u16 {
            if is_plot_glyph(buf[(x, y)].symbol()) {
                left_hits += 1;
            }
        }
        for x in 65..=96u16 {
            if is_plot_glyph(buf[(x, y)].symbol()) {
                right_hits += 1;
            }
        }
    }
    assert_eq!(left_hits, 0, "partial history leaked into the left side of the chart");
    assert!(right_hits > 10, "expected plot glyphs hugging the right edge, found {right_hits}");
}

#[test]
fn options_view_renders_and_masks_token() {
    let opts = apctui::options::Notifications {
        enabled: true,
        pushbullet_token: "o.abcdefghijklmnop".into(),
        ..Default::default()
    };
    let out = render_with(false, 100, 30, |a| a.test_open_options(opts.clone()));
    assert!(out.contains("options"));
    assert!(out.contains("pushbullet token"));
    assert!(out.contains("o.ab"), "shows token head for recognition");
    assert!(
        !out.contains("abcdefghijklmnop"),
        "full token must never be displayed"
    );
    assert!(out.contains("send test notification"));
    assert!(out.contains("active: pushes will send"));
}

#[test]
fn options_view_basic_mode_is_pure_ascii() {
    let opts = apctui::options::Notifications {
        enabled: true,
        pushbullet_token: "o.abcdefghijklmnop".into(),
        ..Default::default()
    };
    let out = render_with(true, 100, 30, |a| a.test_open_options(opts.clone()));
    let non_ascii: Vec<char> = out.chars().filter(|c| !c.is_ascii()).collect();
    assert!(
        non_ascii.is_empty(),
        "options leaked non-ASCII in basic mode: {:?}",
        non_ascii.iter().take(10).collect::<String>()
    );
}

#[test]
fn options_warns_when_enabled_without_token() {
    let opts = apctui::options::Notifications { enabled: true, ..Default::default() };
    let out = render_with(false, 100, 30, |a| a.test_open_options(opts.clone()));
    assert!(out.contains("no token: nothing will send"));
}

#[test]
fn options_confirm_modal_renders_and_is_ascii_clean_in_basic() {
    let opts = apctui::options::Notifications { enabled: true, ..Default::default() };
    let rich = render_with(false, 100, 30, |a| {
        a.test_open_options(opts.clone());
        a.options.as_mut().unwrap().confirm_close = true;
    });
    assert!(rich.contains("unsaved changes"));
    assert!(rich.contains("save and close"));
    assert!(rich.contains("discard changes"));
    assert!(rich.contains("keep editing"));

    let basic = render_with(true, 100, 30, |a| {
        a.test_open_options(opts.clone());
        a.options.as_mut().unwrap().confirm_close = true;
    });
    let non_ascii: Vec<char> = basic.chars().filter(|c| !c.is_ascii()).collect();
    assert!(non_ascii.is_empty(), "confirm modal leaked non-ASCII: {:?}", non_ascii.iter().take(10).collect::<String>());
}

#[test]
fn header_notify_indicator_reflects_armed_state() {
    // disarmed (fixture defaults): no indicator
    let out = render_with(false, 100, 26, |a| a.test_set_view(View::Dashboard));
    assert!(!out.contains("notify on"), "indicator shown while disarmed");

    // armed (enabled + token): indicator present
    let refs = vec![apctui::registry::UpsRef {
        name: "rack-main".into(),
        addr: "127.0.0.1:3551".into(),
    }];
    let opts = apctui::options::Notifications {
        enabled: true,
        pushbullet_token: "o.x".into(),
        ..Default::default()
    };
    let theme = Theme::new(ColorMode::Truecolor, false);
    let app = App::new(&refs, false, opts);
    let backend = TestBackend::new(100, 26);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app, &theme)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(out.contains("notify on"), "armed notifier must show in header");
}

#[test]
fn clientgen_warns_on_loopback_master() {
    let out = render_with(false, 110, 30, |a| {
        a.test_open_clientgen();
        a.clientgen.as_mut().unwrap().tabs[0].params.master_addr = "127.0.0.1:3551".into();
    });
    assert!(out.contains("master is loopback"), "warning missing for 127.0.0.1");

    let out = render_with(false, 110, 30, |a| {
        a.test_open_clientgen(); // fixture seeds 192.168.1.10 - reachable
    });
    assert!(!out.contains("master is loopback"), "false warning on reachable master");

    // basic mode purity with the warning showing
    let basic = render_with(true, 110, 30, |a| {
        a.test_open_clientgen();
        a.clientgen.as_mut().unwrap().tabs[0].params.master_addr = "localhost:3551".into();
    });
    assert!(basic.contains("master is loopback"));
    let non_ascii: Vec<char> = basic.chars().filter(|c| !c.is_ascii()).collect();
    assert!(non_ascii.is_empty(), "warning leaked non-ASCII: {:?}", non_ascii.iter().take(8).collect::<String>());
}
