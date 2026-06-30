// SPDX-License-Identifier: GPL-3.0-or-later
//! Services view: list apcupsd instances, confirm + run systemd actions.

use super::{App, ServicesState, View};
use crate::service;
use ratatui::crossterm::event::KeyCode;
use std::time::Instant;

impl App {
    // ---- services ----
    pub(super) fn open_services(&mut self) {
        let instances = service::discover();
        if instances.is_empty() {
            self.toast_err("no apcupsd configs found in /etc/apcupsd");
            return;
        }
        self.services = Some(ServicesState { instances, cursor: 0, confirm: None });
        self.last_discovery = Some(Instant::now());
        self.goto(View::Services);
    }

    pub(super) fn services_key(&mut self, code: KeyCode) {
        let Some(sv) = &mut self.services else { return };
        if sv.confirm.is_some() {
            match code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let (action, name) = sv.confirm.take().unwrap();
                    self.run_service(action, &name);
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    sv.confirm = None;
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.services = None;
                self.view = View::Dashboard;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if sv.cursor + 1 < sv.instances.len() {
                    sv.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                sv.cursor = sv.cursor.saturating_sub(1);
            }
            KeyCode::Char('r') => self.confirm_service(service::ServiceAction::Restart),
            KeyCode::Char('S') => self.confirm_service(service::ServiceAction::Start),
            KeyCode::Char('x') => self.confirm_service(service::ServiceAction::Stop),
            KeyCode::Char('e') => self.confirm_service(service::ServiceAction::Enable),
            KeyCode::Char('d') => self.confirm_service(service::ServiceAction::Disable),
            KeyCode::Char('R') => {
                self.services = Some(ServicesState {
                    instances: service::discover(),
                    cursor: sv.cursor.min(usize::MAX),
                    confirm: None,
                });
                self.toast_info("rescanned instances");
            }
            _ => {}
        }
    }

    fn confirm_service(&mut self, action: service::ServiceAction) {
        let Some(sv) = &mut self.services else { return };
        let Some(inst) = sv.instances.get(sv.cursor) else { return };
        if inst.name == "apcupsd" {
            self.toast_err("the stock apcupsd.service is managed outside apctui");
            return;
        }
        sv.confirm = Some((action, inst.name.clone()));
    }

    fn run_service(&mut self, action: service::ServiceAction, name: &str) {
        match crate::privileged::run_service_action(action, name) {
            Ok(()) => {
                self.toast_ok(format!("{} apcupsd@{name}", action.verb()));
                // refresh state
                if let Some(sv) = &mut self.services {
                    let cursor = sv.cursor;
                    sv.instances = service::discover();
                    sv.cursor = cursor.min(sv.instances.len().saturating_sub(1));
                }
            }
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("action failed").to_string();
                self.toast_err(first);
            }
        }
    }
}
