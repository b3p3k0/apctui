// SPDX-License-Identifier: GPL-3.0-or-later
//! Units view: add/remove LAN UPS endpoints; persisted, applied on restart.

use super::{AddField, AddForm, App, UnitKind, UnitRow, UnitsState, View};
use ratatui::crossterm::event::KeyCode;

impl App {
    // ---- units (add / remove LAN endpoints; persisted, applied on restart) ----
    pub(super) fn open_units(&mut self) {
        let rows = self.build_unit_rows();
        self.units = Some(UnitsState { rows, cursor: 0, form: None, confirm_remove: None });
        self.goto(View::Units);
    }

    /// Build the Units list: currently-monitored units tagged Local vs Config,
    /// plus config entries not yet monitored (added this session, "pending").
    fn build_unit_rows(&self) -> Vec<UnitRow> {
        use std::collections::HashSet;
        let configured = crate::registry::configured_ups();
        let config_names: HashSet<&str> = configured.iter().map(|u| u.name.as_str()).collect();
        let monitored: HashSet<&str> = self.upses.iter().map(|p| p.name.as_str()).collect();
        let mut rows: Vec<UnitRow> = self
            .upses
            .iter()
            .map(|p| UnitRow {
                name: p.name.clone(),
                addr: p.addr.clone(),
                kind: if config_names.contains(p.name.as_str()) {
                    UnitKind::Config
                } else {
                    UnitKind::Local
                },
                pending: false,
            })
            .collect();
        for u in &configured {
            if !monitored.contains(u.name.as_str()) {
                rows.push(UnitRow {
                    name: u.name.clone(),
                    addr: u.addr.clone(),
                    kind: UnitKind::Config,
                    pending: true,
                });
            }
        }
        rows
    }

    fn refresh_units(&mut self) {
        let rows = self.build_unit_rows();
        if let Some(u) = &mut self.units {
            u.cursor = u.cursor.min(rows.len().saturating_sub(1));
            u.rows = rows;
        }
    }

    pub(super) fn units_key(&mut self, code: KeyCode) {
        // Remove-confirmation modal takes priority.
        let confirming = self.units.as_ref().and_then(|u| u.confirm_remove.clone());
        if let Some(name) = confirming {
            match code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(u) = &mut self.units {
                        u.confirm_remove = None;
                    }
                    self.units_remove(&name);
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    if let Some(u) = &mut self.units {
                        u.confirm_remove = None;
                    }
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.units = None;
                self.view = View::Dashboard;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(u) = &mut self.units {
                    if u.cursor + 1 < u.rows.len() {
                        u.cursor += 1;
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(u) = &mut self.units {
                    u.cursor = u.cursor.saturating_sub(1);
                }
            }
            KeyCode::Char('a') => {
                if let Some(u) = &mut self.units {
                    u.form = Some(AddForm {
                        field: AddField::Name,
                        name: String::new(),
                        host: String::new(),
                    });
                }
            }
            KeyCode::Char('x') => {
                let target = self
                    .units
                    .as_ref()
                    .and_then(|u| u.rows.get(u.cursor))
                    .map(|r| (r.kind, r.name.clone()));
                match target {
                    Some((UnitKind::Config, name)) => {
                        if let Some(u) = &mut self.units {
                            u.confirm_remove = Some(name);
                        }
                    }
                    Some((UnitKind::Local, _)) => {
                        self.toast_info("auto-detected local unit; manage it via discovery")
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    pub(super) fn units_handle_text(&mut self, code: KeyCode) {
        // Submit/cancel reach beyond the form borrow, so handle them first.
        match code {
            KeyCode::Esc => {
                if let Some(u) = &mut self.units {
                    u.form = None;
                }
                return;
            }
            KeyCode::Enter => {
                self.units_submit();
                return;
            }
            _ => {}
        }
        let Some(u) = &mut self.units else { return };
        let Some(form) = &mut u.form else { return };
        match code {
            KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                form.field = match form.field {
                    AddField::Name => AddField::Host,
                    AddField::Host => AddField::Name,
                };
            }
            KeyCode::Backspace => match form.field {
                AddField::Name => {
                    form.name.pop();
                }
                AddField::Host => {
                    form.host.pop();
                }
            },
            KeyCode::Char(c) => match form.field {
                AddField::Name => form.name.push(c),
                AddField::Host => form.host.push(c),
            },
            _ => {}
        }
    }

    fn units_submit(&mut self) {
        let Some((name, host)) = self
            .units
            .as_ref()
            .and_then(|u| u.form.as_ref())
            .map(|f| (f.name.trim().to_string(), f.host.clone()))
        else {
            return;
        };
        if name.is_empty() {
            self.toast_err("name is required");
            return;
        }
        match crate::options::add_ups(&name, &host) {
            Ok(_) => {
                if let Some(u) = &mut self.units {
                    u.form = None;
                }
                self.refresh_units();
                self.toast_ok(format!("added {name} — restart apctui to monitor it"));
            }
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("add failed").to_string();
                self.toast_err(first);
            }
        }
    }

    fn units_remove(&mut self, name: &str) {
        match crate::options::remove_ups(name) {
            Ok(true) => {
                self.refresh_units();
                self.toast_ok(format!("removed {name} — restart to apply"));
            }
            Ok(false) => self.toast_err(format!("{name} not found in config")),
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("remove failed").to_string();
                self.toast_err(first);
            }
        }
    }

    /// Read-only Units state access (test support).
    pub fn units_ref(&self) -> Option<&UnitsState> {
        self.units.as_ref()
    }

    /// Open the Units view (test support).
    pub fn test_open_units(&mut self) {
        self.open_units();
    }
}
