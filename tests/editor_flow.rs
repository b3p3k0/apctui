// SPDX-License-Identifier: GPL-3.0-or-later
//! Exercises editor state transitions without a terminal.
use apctui::app::{App, View};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

fn key(app: &mut App, c: KeyCode) {
    app.on_key(c, KeyModifiers::empty());
}

#[test]
fn editing_absent_directive_adds_it() {
    let mut app = App::test_fixture(false);
    // config without ONBATTERYDELAY (a common directive -> shown as absent)
    app.test_open_editor("UPSNAME x\nUPSCABLE usb\nUPSTYPE usb\nDEVICE\nNETSERVER on\nNISPORT 3551\n");
    assert_eq!(app.view, View::Editor);

    // move cursor to ONBATTERYDELAY and set a value
    // find its index
    let idx = {
        let ed = app.editor_ref().unwrap();
        ed.tab().fields.iter().position(|f| f.key == "ONBATTERYDELAY").expect("field present")
    };
    for _ in 0..idx { key(&mut app, KeyCode::Char('j')); }
    key(&mut app, KeyCode::Enter);          // begin edit
    for c in "6".chars() { key(&mut app, KeyCode::Char(c)); }
    key(&mut app, KeyCode::Enter);          // commit

    let ed = app.editor_ref().unwrap();
    assert_eq!(ed.tab().working.get("ONBATTERYDELAY"), Some("6"));
    assert!(ed.tab().working.serialize().contains("ONBATTERYDELAY 6"));
}

#[test]
fn toggling_bool_and_cycling_enum() {
    let mut app = App::test_fixture(false);
    app.test_open_editor("UPSNAME x\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nNETSERVER on\nNISPORT 3551\n");

    // NETSERVER is a bool currently "on"; find and toggle via space
    let idx = app.editor_ref().unwrap().tab().fields.iter().position(|f| f.key == "NETSERVER").unwrap();
    for _ in 0..idx { key(&mut app, KeyCode::Char('j')); }
    key(&mut app, KeyCode::Char(' '));  // toggle -> off
    assert_eq!(app.editor_ref().unwrap().tab().working.get("NETSERVER"), Some("off"));

    // UPSTYPE enum cycle
    let idx = app.editor_ref().unwrap().tab().fields.iter().position(|f| f.key == "UPSTYPE").unwrap();
    // reset cursor to top then descend
    while app.editor_ref().unwrap().tab().cursor > 0 { key(&mut app, KeyCode::Char('k')); }
    for _ in 0..idx { key(&mut app, KeyCode::Char('j')); }
    let before = app.editor_ref().unwrap().tab().working.get("UPSTYPE").unwrap().to_string();
    key(&mut app, KeyCode::Char(' '));  // cycle
    let after = app.editor_ref().unwrap().tab().working.get("UPSTYPE").unwrap().to_string();
    assert_ne!(before, after);
}

#[test]
fn invalid_int_produces_error_finding() {
    let mut app = App::test_fixture(false);
    app.test_open_editor("UPSNAME x\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nNISPORT 3551\nBATTERYLEVEL 10\n");
    let idx = app.editor_ref().unwrap().tab().fields.iter().position(|f| f.key == "NISPORT").unwrap();
    for _ in 0..idx { key(&mut app, KeyCode::Char('j')); }
    key(&mut app, KeyCode::Enter);
    // clear "3551" then type a bad port
    for _ in 0..5 { key(&mut app, KeyCode::Backspace); }
    for c in "99999".chars() { key(&mut app, KeyCode::Char(c)); }
    key(&mut app, KeyCode::Enter);
    let ed = app.editor_ref().unwrap();
    assert!(ed.tab().findings.iter().any(|f| f.key.as_deref() == Some("NISPORT")));
}


#[test]
fn tab_switching_moves_between_units() {
    let mut app = App::test_fixture(false);
    app.test_open_editor_tabs(&[
        ("apc0", "UPSNAME apc0\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nNISPORT 3551\n"),
        ("apc1", "UPSNAME apc1\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nNISPORT 3552\n"),
    ]);
    assert_eq!(app.editor_ref().unwrap().tab().instance, "apc0");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.editor_ref().unwrap().tab().instance, "apc1");
    key(&mut app, KeyCode::Tab); // wraps
    assert_eq!(app.editor_ref().unwrap().tab().instance, "apc0");
    key(&mut app, KeyCode::BackTab); // wraps backward
    assert_eq!(app.editor_ref().unwrap().tab().instance, "apc1");
}

#[test]
fn edits_are_per_tab() {
    let mut app = App::test_fixture(false);
    app.test_open_editor_tabs(&[
        ("apc0", "UPSNAME apc0\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nBATTERYLEVEL 10\nNISPORT 3551\n"),
        ("apc1", "UPSNAME apc1\nUPSTYPE usb\nUPSCABLE usb\nDEVICE\nBATTERYLEVEL 10\nNISPORT 3552\n"),
    ]);
    // edit BATTERYLEVEL on tab 0
    let idx = app.editor_ref().unwrap().tab().fields.iter().position(|f| f.key == "BATTERYLEVEL").unwrap();
    for _ in 0..idx { key(&mut app, KeyCode::Char('j')); }
    key(&mut app, KeyCode::Enter);
    for _ in 0..2 { key(&mut app, KeyCode::Backspace); }
    for c in "25".chars() { key(&mut app, KeyCode::Char(c)); }
    key(&mut app, KeyCode::Enter);
    assert!(app.editor_ref().unwrap().tab().dirty());
    // tab 1 untouched
    key(&mut app, KeyCode::Tab);
    assert!(!app.editor_ref().unwrap().tab().dirty());
    assert_eq!(app.editor_ref().unwrap().tab().working.get("BATTERYLEVEL"), Some("10"));
}
