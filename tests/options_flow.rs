// SPDX-License-Identifier: GPL-3.0-or-later
//! Key-driven options form flow. Regression coverage for the routing bug
//! where edit-mode keystrokes fell through to the command handler ('q'
//! while typing a token closed the view).

use apctui::app::{App, View};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

fn key(app: &mut App, code: KeyCode) {
    app.on_key(code, KeyModifiers::NONE);
}

fn open_options(app: &mut App) {
    app.test_open_options(apctui::options::Notifications::default());
}

#[test]
fn typing_a_token_with_q_does_not_quit_the_view() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    // move to the token field (index 2) and start editing
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    // type a token full of command keys: q, s, t, j, k, space
    for c in "o.qstjk q".chars() {
        key(&mut app, KeyCode::Char(c));
        assert_eq!(app.view, View::Options, "view changed while typing {c:?}");
    }
    key(&mut app, KeyCode::Enter);
    let op = app.options_ref().expect("options state alive");
    assert_eq!(op.working.pushbullet_token, "o.qstjk q");
    assert!(!op.editing, "edit committed");
    assert_eq!(app.view, View::Options);
}

#[test]
fn esc_cancels_edit_without_closing_view() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Esc); // cancels the edit...
    assert_eq!(app.view, View::Options, "first esc must only cancel the edit");
    let op = app.options_ref().unwrap();
    assert!(op.working.pushbullet_token.is_empty(), "cancelled edit must not commit");
    key(&mut app, KeyCode::Esc); // ...second esc closes the view
    assert_eq!(app.view, View::Dashboard);
}

#[test]
fn space_toggles_booleans() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    // cursor 0 = enabled toggle
    assert!(!app.options_ref().unwrap().working.enabled);
    key(&mut app, KeyCode::Char(' '));
    assert!(app.options_ref().unwrap().working.enabled);
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.options_ref().unwrap().working.enabled);
}

#[test]
fn cooldown_rejects_non_numeric_input() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    for _ in 0..7 {
        key(&mut app, KeyCode::Char('j'));
    }
    key(&mut app, KeyCode::Enter);
    for _ in 0..3 {
        key(&mut app, KeyCode::Backspace); // clear "60"
    }
    for c in "abc".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    let op = app.options_ref().unwrap();
    assert_eq!(op.working.cooldown_secs, 60, "invalid input must not change the value");
}

// ---- unsaved-changes prompt ----

fn make_dirty(app: &mut App) {
    // cursor starts at 0 (enabled toggle); space flips it
    key(app, KeyCode::Char(' '));
}

#[test]
fn clean_close_needs_no_prompt() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.view, View::Dashboard, "no changes, esc closes immediately");
}

#[test]
fn dirty_close_raises_prompt_and_esc_returns_to_form() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    make_dirty(&mut app);
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.view, View::Options, "dirty q must not close");
    assert!(app.options_ref().unwrap().confirm_close, "prompt raised");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.view, View::Options);
    assert!(!app.options_ref().unwrap().confirm_close, "esc returns to the form");
    // edits preserved
    assert!(app.options_ref().unwrap().working.enabled);
}

#[test]
fn prompt_keys_do_not_leak_to_the_form() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    make_dirty(&mut app);
    key(&mut app, KeyCode::Esc); // raise prompt
    let cursor_before = app.options_ref().unwrap().cursor;
    key(&mut app, KeyCode::Char('j')); // would move cursor if it leaked
    key(&mut app, KeyCode::Char('t')); // would fire a test push if it leaked
    assert_eq!(app.options_ref().unwrap().cursor, cursor_before);
    assert!(app.options_ref().unwrap().confirm_close, "prompt still up");
}

#[test]
fn discard_closes_without_applying() {
    let mut app = App::test_fixture(false);
    open_options(&mut app);
    make_dirty(&mut app);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.view, View::Dashboard);
    assert!(!app.notify_opts.enabled, "discarded change must not reach live settings");
}

#[test]
fn save_from_prompt_persists_and_closes() {
    // isolate the config write
    let dir = std::env::temp_dir().join(format!("apctui-prompt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &dir);

    let mut app = App::test_fixture(false);
    open_options(&mut app);
    make_dirty(&mut app);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.view, View::Dashboard);
    assert!(app.notify_opts.enabled, "saved change must reach live settings");
    let written = std::fs::read_to_string(dir.join("apctui").join("config.toml")).unwrap();
    assert!(written.contains("enabled = true"));

    std::env::remove_var("XDG_CONFIG_HOME");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_twice_keeps_the_notifier_armed() {
    // Regression: rebuilding on save must release this process's own lock
    // before re-acquiring, or the instance strands itself in standby.
    let dir = std::env::temp_dir().join(format!("apctui-resave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &dir);

    let mut app = App::test_fixture(false);
    open_options(&mut app);
    key(&mut app, KeyCode::Char(' ')); // enable
    // token
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    for c in "o.x".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('s'));
    assert!(app.notifier_active(), "armed after first save");
    // change cooldown, save again
    for _ in 0..5 {
        key(&mut app, KeyCode::Char('j'));
    }
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('0'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('s'));
    assert!(app.notifier_active(), "must stay armed across re-saves (lock re-acquired)");

    std::env::remove_var("XDG_CONFIG_HOME");
    std::fs::remove_dir_all(&dir).ok();
}
