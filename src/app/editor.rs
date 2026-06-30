// SPDX-License-Identifier: GPL-3.0-or-later
//! Config editor view: per-instance tabs, field edit, diff, privileged save.

use super::{App, EditorState, View, load_editor_tab};
use crate::config;
use crate::service;
use ratatui::crossterm::event::KeyCode;

impl App {
    // ---- editor ----
    pub(super) fn open_editor(&mut self) {
        // One tab per local instance (skips the stock unit and CGI confs).
        let mut tabs = Vec::new();
        for inst in service::discover() {
            if inst.name == "apcupsd" {
                continue;
            }
            if let Some(tab) = load_editor_tab(&inst.name) {
                tabs.push(tab);
            }
        }
        // Fallback: not a managed host (e.g. monitoring remotes) — try the
        // selected panel's conf directly.
        if tabs.is_empty() {
            if let Some(panel) = self.upses.get(self.selected) {
                if let Some(tab) = load_editor_tab(&panel.name) {
                    tabs.push(tab);
                }
            }
        }
        if tabs.is_empty() {
            self.toast_err(format!("no editable configs found in {}", service::CONF_DIR));
            return;
        }
        // Start on the tab matching the selected unit, if present.
        let active = self
            .upses
            .get(self.selected)
            .and_then(|p| tabs.iter().position(|t| t.instance == p.name))
            .unwrap_or(0);
        self.editor = Some(EditorState {
            tabs,
            active,
            editing: false,
            edit_buffer: String::new(),
            show_diff: false,
        });
        self.goto(View::Editor);
    }

    pub(super) fn editor_key(&mut self, code: KeyCode) {
        let Some(ed) = &mut self.editor else { return };
        if ed.show_diff {
            match code {
                KeyCode::Esc | KeyCode::Char('d') => ed.show_diff = false,
                KeyCode::Char('s') => self.editor_save(),
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                let dirty = ed.tabs.iter().filter(|t| t.dirty()).count();
                if dirty > 0 {
                    self.toast_info(format!(
                        "discarded unsaved changes in {dirty} tab{}",
                        if dirty == 1 { "" } else { "s" }
                    ));
                }
                self.editor = None;
                self.view = View::Dashboard;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                ed.active = (ed.active + 1) % ed.tabs.len();
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                ed.active = (ed.active + ed.tabs.len() - 1) % ed.tabs.len();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let t = ed.tab_mut();
                if t.cursor + 1 < t.fields.len() {
                    t.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let t = ed.tab_mut();
                t.cursor = t.cursor.saturating_sub(1);
            }
            KeyCode::Enter => self.editor_begin_edit(),
            KeyCode::Char(' ') => self.editor_toggle_or_cycle(),
            KeyCode::Char('d') => {
                self.editor_revalidate();
                if let Some(ed) = &mut self.editor {
                    ed.show_diff = true;
                }
            }
            KeyCode::Char('s') => self.editor_save(),
            _ => {}
        }
    }

    fn editor_begin_edit(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        let cursor = ed.tab().cursor;
        let Some(field) = ed.tab().fields.get(cursor) else { return };
        match &field.kind {
            config::Kind::Bool | config::Kind::Enum(_) => {
                self.editor_toggle_or_cycle();
            }
            config::Kind::Text | config::Kind::Int { .. } => {
                ed.edit_buffer = field.value.clone();
                ed.editing = true;
            }
        }
    }

    fn editor_toggle_or_cycle(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        let t = ed.tab_mut();
        let cursor = t.cursor;
        let Some(field) = t.fields.get_mut(cursor) else { return };
        match &field.kind {
            config::Kind::Bool => {
                let next = if field.value.eq_ignore_ascii_case("on") { "off" } else { "on" };
                field.value = next.to_string();
                field.present = true;
            }
            config::Kind::Enum(opts) => {
                let cur = opts.iter().position(|o| o.eq_ignore_ascii_case(&field.value));
                let next = match cur {
                    Some(i) => opts[(i + 1) % opts.len()],
                    None => opts[0],
                };
                field.value = next.to_string();
                field.present = true;
            }
            _ => {}
        }
        let key = field.key.clone();
        let val = field.value.clone();
        t.working.set(&key, &val);
        self.editor_revalidate();
    }

    pub(super) fn editor_handle_text(&mut self, code: KeyCode) {
        let Some(ed) = &mut self.editor else { return };
        match code {
            KeyCode::Esc => {
                ed.editing = false;
                ed.edit_buffer.clear();
            }
            KeyCode::Enter => {
                let buf = ed.edit_buffer.clone();
                let t = ed.tab_mut();
                let cursor = t.cursor;
                if let Some(field) = t.fields.get_mut(cursor) {
                    field.value = buf.clone();
                    field.present = true;
                    let key = field.key.clone();
                    t.working.set(&key, &buf);
                }
                ed.editing = false;
                ed.edit_buffer.clear();
                self.editor_revalidate();
            }
            KeyCode::Backspace => {
                ed.edit_buffer.pop();
            }
            KeyCode::Char(c) => {
                ed.edit_buffer.push(c);
            }
            _ => {}
        }
    }

    fn editor_revalidate(&mut self) {
        if let Some(ed) = &mut self.editor {
            let t = ed.tab_mut();
            t.findings = config::validate(&t.working);
        }
    }

    fn editor_save(&mut self) {
        let Some(ed) = &self.editor else { return };
        let t = ed.tab();
        if config::validate::has_errors(&t.findings) {
            self.toast_err("fix errors before saving (see banner)");
            return;
        }
        if !t.dirty() {
            self.toast_info("no changes to save in this tab");
            return;
        }
        let plan = crate::privileged::ApplyPlan {
            dest: t.path.clone(),
            new_contents: t.working.serialize(),
            restart: Some(t.instance.clone()),
            service: None,
        };
        let instance = t.instance.clone();
        match crate::privileged::run_apply(&plan) {
            Ok(()) => {
                self.toast_ok(format!("saved & restarted apcupsd@{instance}"));
                // Mark the tab clean and stay in the editor.
                if let Some(ed) = &mut self.editor {
                    let t = ed.tab_mut();
                    t.original = t.working.clone();
                    ed.show_diff = false;
                }
            }
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("apply failed").to_string();
                self.toast_err(first);
            }
        }
    }
}
