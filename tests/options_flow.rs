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
