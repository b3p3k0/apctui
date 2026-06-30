// SPDX-License-Identifier: GPL-3.0-or-later
//! Network-client config generator view: per-unit tabs, field edit, write bundle.

use super::{App, ClientGenState, ClientGenTab, View, clientgen_field_value, clientgen_set_field, dirs_output};
use crate::config::ConfigFile;
use crate::service;
use ratatui::crossterm::event::KeyCode;

impl App {
    // ---- client gen ----
    pub(super) fn open_clientgen(&mut self) {
        // One tab per monitored unit, each seeded from its on-disk config
        // when available. The master address apctui polls is usually
        // loopback - meaningless to a network client - so substitute this
        // host's detected LAN address (see netutil for range priority).
        let lan = crate::netutil::lan_ip().map(|ip| ip.to_string());
        let mut tabs = Vec::new();
        for panel in &self.upses {
            let mut params = crate::clientgen::ClientParams::default();
            let path = std::path::Path::new(service::CONF_DIR)
                .join(format!("{}.conf", panel.name));
            let reachable_host = |h: &str| -> String {
                let loopback = h == "localhost" || h.starts_with("127.");
                match (&lan, loopback) {
                    (Some(ip), true) => ip.clone(),
                    _ => h.to_string(),
                }
            };
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let cf = ConfigFile::parse(&raw);
                let port = cf.get("NISPORT").unwrap_or("3551").to_string();
                let host = reachable_host(panel.addr.split(':').next().unwrap_or("127.0.0.1"));
                params = crate::clientgen::suggest_from_master(&cf, &format!("{host}:{port}"));
            } else {
                params.ups_name = panel.name.clone();
                let host = reachable_host(panel.addr.split(':').next().unwrap_or("127.0.0.1"));
                let port = panel.addr.split(':').nth(1).unwrap_or("3551");
                params.master_addr = format!("{host}:{port}");
            }
            let preview = crate::clientgen::render_conf(&params);
            tabs.push(ClientGenTab {
                instance: panel.name.clone(),
                params,
                preview,
                saved_path: None,
            });
        }
        if tabs.is_empty() {
            self.toast_err("no units to generate client configs for");
            return;
        }
        let active = self.selected.min(tabs.len() - 1);
        self.clientgen = Some(ClientGenState {
            tabs,
            active,
            cursor: 0,
            editing: false,
            edit_buffer: String::new(),
        });
        self.goto(View::ClientGen);
    }

    fn clientgen_fields() -> usize { 5 }

    pub(super) fn clientgen_key(&mut self, code: KeyCode) {
        let Some(cg) = &mut self.clientgen else { return };
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.clientgen = None;
                self.view = View::Dashboard;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                cg.active = (cg.active + 1) % cg.tabs.len();
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                cg.active = (cg.active + cg.tabs.len() - 1) % cg.tabs.len();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if cg.cursor + 1 < Self::clientgen_fields() {
                    cg.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                cg.cursor = cg.cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                cg.edit_buffer = clientgen_field_value(&cg.tab().params, cg.cursor);
                cg.editing = true;
            }
            KeyCode::Char('w') => self.clientgen_save(),
            _ => {}
        }
    }

    pub(super) fn clientgen_handle_text(&mut self, code: KeyCode) {
        let Some(cg) = &mut self.clientgen else { return };
        match code {
            KeyCode::Esc => {
                cg.editing = false;
                cg.edit_buffer.clear();
            }
            KeyCode::Enter => {
                let buf = cg.edit_buffer.clone();
                let cursor = cg.cursor;
                let active = cg.active;
                let tab = &mut cg.tabs[active];
                clientgen_set_field(&mut tab.params, cursor, &buf);
                tab.preview = crate::clientgen::render_conf(&tab.params);
                cg.editing = false;
                cg.edit_buffer.clear();
            }
            KeyCode::Backspace => {
                cg.edit_buffer.pop();
            }
            KeyCode::Char(c) => cg.edit_buffer.push(c),
            _ => {}
        }
    }

    fn clientgen_save(&mut self) {
        let Some(cg) = &self.clientgen else { return };
        let t = cg.tab();
        if t.params.master_addr.is_empty() || !t.params.master_addr.contains(':') {
            self.toast_err("set master address as host:port first");
            return;
        }
        let dir = dirs_output();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast_err(format!("cannot create {}: {e}", dir.display()));
            return;
        }
        let conf = crate::clientgen::render_conf(&t.params);
        let readme = crate::clientgen::render_bundle_readme(&t.params);
        let stem = if t.params.ups_name.is_empty() { "ups" } else { &t.params.ups_name };
        let conf_path = dir.join(format!("{stem}-client-apcupsd.conf"));
        let readme_path = dir.join(format!("{stem}-client-INSTALL.txt"));
        let r1 = std::fs::write(&conf_path, conf);
        let r2 = std::fs::write(&readme_path, readme);
        match (r1, r2) {
            (Ok(()), Ok(())) => {
                let p = conf_path.display().to_string();
                if let Some(cg) = &mut self.clientgen {
                    let active = cg.active;
                    cg.tabs[active].saved_path = Some(p);
                }
                self.toast_ok(format!("bundle written to {}", dir.display()));
            }
            _ => self.toast_err("failed writing client bundle"),
        }
    }
}
