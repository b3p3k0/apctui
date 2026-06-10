// SPDX-License-Identifier: GPL-3.0-or-later
//! Transition detection: App::apply -> pending notification events.

use apctui::app::App;
use apctui::nis::UpsStatus;
use apctui::notify::EventKind;
use apctui::poller::Update;
use std::collections::HashMap;

fn status(s: &str) -> UpsStatus {
    let mut fields = HashMap::new();
    fields.insert("STATUS".to_string(), s.to_string());
    fields.insert("LOADPCT".to_string(), "48.0 Percent".to_string());
    fields.insert("BCHARGE".to_string(), "77.0 Percent".to_string());
    fields.insert("TIMELEFT".to_string(), "22.0 Minutes".to_string());
    UpsStatus { fields }
}

fn ok(idx: usize, s: &str) -> Update {
    Update { idx, result: Ok(status(s)) }
}

fn err(idx: usize) -> Update {
    Update { idx, result: Err("connect refused".to_string()) }
}

#[test]
fn first_sample_never_notifies_even_on_battery() {
    let mut app = App::test_fixture(false);
    app.test_take_pending(); // clear anything from fixture setup
    // a third unit would be cleaner, but unit 0 has status yet no on_battery
    // baseline (fixture sets status directly) — first apply sets the baseline
    app.apply(ok(0, "ONBATT"));
    assert!(app.test_take_pending().is_empty(), "startup state must not notify");
    // ...but the NEXT transition does
    app.apply(ok(0, "ONLINE"));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::OnLine);
}

#[test]
fn online_to_onbatt_and_back() {
    let mut app = App::test_fixture(false);
    app.apply(ok(0, "ONLINE"));
    app.test_take_pending();

    app.apply(ok(0, "ONBATT"));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::OnBattery);
    assert_eq!(evs[0].unit, "rack-main");
    assert!(evs[0].detail.contains("load 48%"), "detail: {}", evs[0].detail);
    assert!(evs[0].detail.contains("22 min"), "detail: {}", evs[0].detail);

    // steady ONBATT: no repeat
    app.apply(ok(0, "ONBATT"));
    assert!(app.test_take_pending().is_empty());

    app.apply(ok(0, "ONLINE"));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::OnLine);
}

#[test]
fn comm_lost_needs_three_consecutive_failures() {
    let mut app = App::test_fixture(false);
    app.apply(ok(1, "ONLINE"));
    app.test_take_pending();

    app.apply(err(1));
    app.apply(err(1));
    assert!(app.test_take_pending().is_empty(), "two failures is a blip, not an outage");
    app.apply(err(1));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::CommLost);
    assert_eq!(evs[0].detail, "connect refused");

    // stays lost: no repeat on further failures
    app.apply(err(1));
    assert!(app.test_take_pending().is_empty());

    // recovery
    app.apply(ok(1, "ONLINE"));
    let evs = app.test_take_pending();
    assert_eq!(evs[0].kind, EventKind::CommRestored);
}

#[test]
fn blip_recovery_resets_failure_counter() {
    let mut app = App::test_fixture(false);
    app.apply(ok(0, "ONLINE"));
    app.test_take_pending();
    app.apply(err(0));
    app.apply(err(0));
    app.apply(ok(0, "ONLINE")); // recovered before threshold
    app.apply(err(0));
    app.apply(err(0));
    assert!(app.test_take_pending().is_empty(), "counter must reset after recovery");
}

// ---- daemon-reported COMMLOST (NIS healthy, UPS link down) ----

#[test]
fn daemon_commlost_status_triggers_comm_lost_after_three() {
    let mut app = App::test_fixture(false);
    app.apply(ok(0, "ONLINE"));
    app.test_take_pending();

    app.apply(ok(0, "COMMLOST"));
    app.apply(ok(0, "COMMLOST"));
    assert!(app.test_take_pending().is_empty(), "two COMMLOST samples is a blip");
    app.apply(ok(0, "COMMLOST"));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::CommLost);
    assert!(evs[0].detail.contains("UPS link down"), "detail: {}", evs[0].detail);

    // stays lost: no repeats
    app.apply(ok(0, "COMMLOST"));
    assert!(app.test_take_pending().is_empty());

    // link returns
    app.apply(ok(0, "ONLINE"));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventKind::CommRestored);
}

#[test]
fn commlost_samples_do_not_fake_battery_transitions() {
    let mut app = App::test_fixture(false);
    app.apply(ok(0, "ONBATT"));
    app.test_take_pending();
    // UPS link drops while on battery; stale COMMLOST samples must not
    // produce a phantom "back on line", and the baseline must survive.
    app.apply(ok(0, "COMMLOST"));
    app.apply(ok(0, "COMMLOST"));
    let evs = app.test_take_pending();
    assert!(evs.iter().all(|e| e.kind != EventKind::OnLine), "phantom OnLine from COMMLOST");
    // link returns, still on battery: no transition events either
    app.apply(ok(0, "ONBATT"));
    let evs = app.test_take_pending();
    assert!(evs.iter().all(|e| e.kind != EventKind::OnBattery && e.kind != EventKind::OnLine));
}

#[test]
fn mixed_nis_failures_and_commlost_share_the_counter() {
    let mut app = App::test_fixture(false);
    app.apply(ok(1, "ONLINE"));
    app.test_take_pending();
    app.apply(err(1));
    app.apply(ok(1, "COMMLOST"));
    app.apply(err(1));
    let evs = app.test_take_pending();
    assert_eq!(evs.len(), 1, "three consecutive comm problems of mixed flavor = lost");
    assert_eq!(evs[0].kind, EventKind::CommLost);
}
