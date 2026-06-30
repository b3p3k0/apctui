// SPDX-License-Identifier: GPL-3.0-or-later
//! Key-driven Units view flow: add a LAN unit (persists to config), the
//! routing guard (field keystrokes must not trigger command keys), remove a
//! config unit, and the refusal to remove an auto-discovered local unit.

use apctui::app::{App, View};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

fn key(app: &mut App, code: KeyCode) {
    app.on_key(code, KeyModifiers::NONE);
}
fn typ(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c));
    }
}

// XDG_CONFIG_HOME is process-global; serialize the set/use/restore window and
// use a unique dir per call (parallel env-var race lesson).
static XDG_LOCK: Mutex<()> = Mutex::new(());
static XDG_SEQ: AtomicU32 = AtomicU32::new(0);

fn with_xdg_home<F: FnOnce(&std::path::Path)>(f: F) {
    let _guard = XDG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!(
        "apctui-units-{}-{}",
        std::process::id(),
        XDG_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let old = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    f(&dir);
    match old {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn add_lan_unit_writes_config_and_shows_row() {
    with_xdg_home(|dir| {
        let mut app = App::test_fixture(false);
        app.test_open_units();
        assert_eq!(app.view, View::Units);

        key(&mut app, KeyCode::Char('a')); // open add form
        assert!(app.units_ref().unwrap().form.is_some());
        typ(&mut app, "rack-lan");
        key(&mut app, KeyCode::Tab); // -> host field
        typ(&mut app, "192.168.1.50:3560");
        key(&mut app, KeyCode::Enter); // submit

        assert!(app.units_ref().unwrap().form.is_none(), "form should close on save");
        let cfg = std::fs::read_to_string(dir.join("apctui").join("config.toml")).unwrap();
        assert!(cfg.contains("rack-lan"), "config missing entry: {cfg}");
        assert!(cfg.contains("192.168.1.50:3560"));
        assert!(
            app.units_ref().unwrap().rows.iter().any(|r| r.name == "rack-lan"),
            "new row not shown"
        );
    });
}

#[test]
fn field_keystrokes_do_not_trigger_commands() {
    with_xdg_home(|_dir| {
        let mut app = App::test_fixture(false);
        app.test_open_units();
        key(&mut app, KeyCode::Char('a')); // open form
        // 'q' (quit) and 'x' (remove) must land in the field, not act as commands
        typ(&mut app, "qx");
        let u = app.units_ref().unwrap();
        assert!(u.form.is_some(), "a field keystroke closed the form");
        assert_eq!(u.form.as_ref().unwrap().name, "qx");
        assert_eq!(app.view, View::Units, "a field keystroke changed views");
        assert!(u.confirm_remove.is_none(), "a field keystroke opened remove confirm");
    });
}

#[test]
fn remove_config_unit_deletes_from_config() {
    with_xdg_home(|dir| {
        apctui::options::add_ups("rack-lan", "192.168.1.50:3560").unwrap();
        let mut app = App::test_fixture(false);
        app.test_open_units();

        let pos = app
            .units_ref()
            .unwrap()
            .rows
            .iter()
            .position(|r| r.name == "rack-lan")
            .expect("config row present");
        for _ in 0..pos {
            key(&mut app, KeyCode::Char('j'));
        }
        key(&mut app, KeyCode::Char('x')); // raise confirm
        assert!(app.units_ref().unwrap().confirm_remove.is_some());
        key(&mut app, KeyCode::Char('y')); // confirm

        let cfg = std::fs::read_to_string(dir.join("apctui").join("config.toml")).unwrap();
        assert!(!cfg.contains("rack-lan"), "entry still in config: {cfg}");
    });
}

#[test]
fn cannot_remove_local_unit() {
    with_xdg_home(|_dir| {
        let mut app = App::test_fixture(false);
        app.test_open_units();
        // cursor 0 is an auto-discovered/local fixture unit
        key(&mut app, KeyCode::Char('x'));
        assert!(
            app.units_ref().unwrap().confirm_remove.is_none(),
            "a local unit must not be removable here"
        );
    });
}
