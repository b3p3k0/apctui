// SPDX-License-Identifier: GPL-3.0-or-later
//! Options view: notification settings form, save + notifier rebuild.

use super::{App, OptionsState, View, OPTIONS_FIELDS};
use ratatui::crossterm::event::KeyCode;
use std::time::{Duration, Instant};

impl App {
    // ---- options ----
    pub(super) fn open_options(&mut self) {
        self.options = Some(OptionsState {
            working: self.notify_opts.clone(),
            cursor: 0,
            editing: false,
            edit_buffer: String::new(),
            confirm_close: false,
        });
        self.goto(View::Options);
    }

    pub(super) fn options_key(&mut self, code: KeyCode) {
        let Some(op) = &mut self.options else { return };
        if op.confirm_close {
            match code {
                KeyCode::Char('s') | KeyCode::Enter => {
                    if self.options_save() {
                        self.options = None;
                        self.view = View::Dashboard;
                    } else if let Some(op) = &mut self.options {
                        // save failed: error toast is up; back to the form
                        op.confirm_close = false;
                    }
                }
                KeyCode::Char('d') => {
                    self.options = None;
                    self.view = View::Dashboard;
                    self.toast_info("changes discarded");
                }
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
                    op.confirm_close = false;
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if op.working != self.notify_opts {
                    op.confirm_close = true;
                    return;
                }
                self.options = None;
                self.view = View::Dashboard;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if op.cursor + 1 < OPTIONS_FIELDS {
                    op.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                op.cursor = op.cursor.saturating_sub(1);
            }
            KeyCode::Char(' ') => self.options_toggle(),
            KeyCode::Enter => match op.cursor {
                0 | 1 | 3..=6 => self.options_toggle(),
                2 | 7 => {
                    op.edit_buffer = match op.cursor {
                        2 => op.working.pushbullet_token.clone(),
                        _ => op.working.cooldown_secs.to_string(),
                    };
                    op.editing = true;
                }
                8 => self.options_send_test(),
                _ => {}
            },
            KeyCode::Char('t') => self.options_send_test(),
            KeyCode::Char('s') => {
                let _ = self.options_save();
            }
            _ => {}
        }
    }

    fn options_toggle(&mut self) {
        let Some(op) = &mut self.options else { return };
        match op.cursor {
            0 => op.working.enabled = !op.working.enabled,
            1 => {} // single provider; nothing to cycle yet
            3 => op.working.on_battery = !op.working.on_battery,
            4 => op.working.on_line = !op.working.on_line,
            5 => op.working.comm_lost = !op.working.comm_lost,
            6 => op.working.comm_restored = !op.working.comm_restored,
            _ => {}
        }
    }

    pub(super) fn options_handle_text(&mut self, code: KeyCode) {
        let Some(op) = &mut self.options else { return };
        match code {
            KeyCode::Esc => {
                op.editing = false;
                op.edit_buffer.clear();
            }
            KeyCode::Enter => {
                let buf = op.edit_buffer.clone();
                match op.cursor {
                    2 => op.working.pushbullet_token = buf.trim().to_string(),
                    7 => match buf.trim().parse::<u64>() {
                        Ok(v) => op.working.cooldown_secs = v,
                        Err(_) => {
                            self.toast_err("cooldown must be a whole number of seconds");
                            if let Some(op) = &mut self.options {
                                op.editing = false;
                                op.edit_buffer.clear();
                            }
                            return;
                        }
                    },
                    _ => {}
                }
                op.editing = false;
                op.edit_buffer.clear();
            }
            KeyCode::Backspace => {
                op.edit_buffer.pop();
            }
            KeyCode::Char(c) => op.edit_buffer.push(c),
            _ => {}
        }
    }

    fn options_send_test(&mut self) {
        let Some(op) = &self.options else { return };
        if op.working.pushbullet_token.is_empty() {
            self.toast_err("set a pushbullet token first");
            return;
        }
        // One-shot notifier with the *working* settings so the test reflects
        // what's on screen, saved or not. Forced enabled: testing while the
        // master switch is off is a legitimate setup step.
        let mut probe = op.working.clone();
        probe.enabled = true;
        // No singleton lock: a test push is explicit user action, and the
        // main notifier in this very process usually holds the lock.
        let n = crate::notify::Notifier::spawn_with_lock(&probe, None);
        n.send(crate::notify::NotifyEvent {
            unit: String::new(),
            kind: crate::notify::EventKind::Test,
            detail: "If you can read this, apctui can reach you.".into(),
        });
        // Hand off to the app notifier slot? No — poll this probe inline on
        // the next ticks by stashing it.
        self.test_notifier = Some(n);
        self.toast_info("test push queued...");
    }

    fn options_save(&mut self) -> bool {
        let Some(op) = &self.options else { return false };
        let working = op.working.clone();
        match crate::options::save(&working) {
            Ok(path) => {
                self.notify_opts = working;
                // Drop first: flock is per-process-wide on this file, and
                // plain reassignment evaluates the new spawn (which tries to
                // lock) BEFORE dropping the old holder - permanent standby.
                self.notifier = crate::notify::Notifier::disabled();
                // Reclaiming our own just-released lock: the OS release isn't
                // always visible to an immediate re-acquire under load, which
                // would strand us in standby until tick()'s 10s takeover. Retry
                // briefly (resolves in a few ms) so re-arm is reliable. Bounded
                // and save-action-only, so the UI thread isn't meaningfully held.
                let mut n = crate::notify::Notifier::spawn(&self.notify_opts);
                let start = Instant::now();
                while n.state() == crate::notify::NotifierState::Standby
                    && start.elapsed() < Duration::from_millis(50)
                {
                    std::thread::sleep(Duration::from_millis(2));
                    n = crate::notify::Notifier::spawn(&self.notify_opts);
                }
                self.notifier = n;
                self.toast_ok(format!("saved {}", path.display()));
                true
            }
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("save failed").to_string();
                self.toast_err(first);
                false
            }
        }
    }
}
