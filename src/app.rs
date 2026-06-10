// SPDX-License-Identifier: GPL-3.0-or-later
//! Application state and input handling.
//!
//! The app is a small view stack machine. The dashboard is the root; detail,
//! editor, services, client-gen, events, and help are pushed over it. Each
//! view owns its local state in a struct below. All privileged work is
//! dispatched through `privileged::*` and reported back via `Toast`.

use crate::config::{self, ConfigFile};
use crate::nis::UpsStatus;
use crate::poller::Update;
use crate::registry::UpsRef;
use crate::service::{self, Instance};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const HISTORY: usize = 600; // 10 min at 1 s cadence

pub struct UpsPanel {
    pub name: String,
    pub addr: String,
    pub status: Option<UpsStatus>,
    pub error: Option<String>,
    pub last_ok: Option<Instant>,
    pub load_hist: VecDeque<u64>,
    pub batt_hist: VecDeque<u64>,
}

impl UpsPanel {
    fn new(r: &UpsRef) -> Self {
        Self {
            name: r.name.clone(),
            addr: r.addr.clone(),
            status: None,
            error: None,
            last_ok: None,
            load_hist: VecDeque::with_capacity(HISTORY),
            batt_hist: VecDeque::with_capacity(HISTORY),
        }
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum View {
    Dashboard,
    Detail,
    Editor,
    Services,
    ClientGen,
    Events,
    Help,
}

/// Transient status message shown in the footer/banner.
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub born: Instant,
    pub ttl: Duration,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

impl Toast {
    fn new(text: impl Into<String>, kind: ToastKind) -> Self {
        Toast { text: text.into(), kind, born: Instant::now(), ttl: Duration::from_secs(6) }
    }
    fn alive(&self) -> bool {
        self.born.elapsed() < self.ttl
    }
}

/// One editable field in the config editor, derived from the schema +
/// current file contents.
pub struct EditField {
    pub key: String,
    pub group: config::Group,
    pub value: String,
    /// Present in the on-disk file (vs. a schema default we're offering).
    pub present: bool,
    pub help: String,
    pub kind: config::Kind,
}

/// One unit's config under edit.
pub struct EditorTab {
    pub instance: String,
    pub path: std::path::PathBuf,
    pub original: ConfigFile,
    pub working: ConfigFile,
    pub fields: Vec<EditField>,
    pub cursor: usize,
    pub findings: Vec<config::Finding>,
}

impl EditorTab {
    pub fn dirty(&self) -> bool {
        self.working.serialize() != self.original.serialize()
    }
}

/// The centralized config editor: one tab per local instance.
pub struct EditorState {
    pub tabs: Vec<EditorTab>,
    pub active: usize,
    /// True while the selected field is being text-edited.
    pub editing: bool,
    pub edit_buffer: String,
    pub show_diff: bool,
}

impl EditorState {
    pub fn tab(&self) -> &EditorTab {
        &self.tabs[self.active]
    }
    fn tab_mut(&mut self) -> &mut EditorTab {
        &mut self.tabs[self.active]
    }
}

pub struct ServicesState {
    pub instances: Vec<Instance>,
    pub cursor: usize,
    /// Pending confirmation: (action, instance name).
    pub confirm: Option<(service::ServiceAction, String)>,
}

/// One unit's client-config parameters.
pub struct ClientGenTab {
    pub instance: String,
    pub params: crate::clientgen::ClientParams,
    pub preview: String,
    pub saved_path: Option<String>,
}

pub struct ClientGenState {
    pub tabs: Vec<ClientGenTab>,
    pub active: usize,
    pub cursor: usize,
    pub editing: bool,
    pub edit_buffer: String,
}

impl ClientGenState {
    pub fn tab(&self) -> &ClientGenTab {
        &self.tabs[self.active]
    }
}

pub struct App {
    pub upses: Vec<UpsPanel>,
    pub selected: usize,
    pub view: View,
    pub prev_view: View,
    pub paused: bool,
    pub basic: bool,
    pub events: Vec<String>,
    pub events_scroll: u16,
    pub should_quit: bool,
    pub toast: Option<Toast>,
    pub editor: Option<EditorState>,
    pub services: Option<ServicesState>,
    pub clientgen: Option<ClientGenState>,
    pub detail_scroll: u16,
    last_discovery: Option<Instant>,
}

impl App {
    pub fn new(refs: &[UpsRef], basic: bool) -> Self {
        Self {
            upses: refs.iter().map(UpsPanel::new).collect(),
            selected: 0,
            view: View::Dashboard,
            prev_view: View::Dashboard,
            paused: false,
            basic,
            events: Vec::new(),
            events_scroll: 0,
            should_quit: false,
            toast: None,
            editor: None,
            services: None,
            clientgen: None,
            detail_scroll: 0,
            last_discovery: None,
        }
    }

    pub fn apply(&mut self, u: Update) {
        if self.paused {
            return;
        }
        let Some(panel) = self.upses.get_mut(u.idx) else { return };
        match u.result {
            Ok(status) => {
                let load = status.num("LOADPCT").unwrap_or(0.0).clamp(0.0, 100.0);
                let batt = status.num("BCHARGE").unwrap_or(0.0).clamp(0.0, 100.0);
                push_hist(&mut panel.load_hist, load.round() as u64);
                push_hist(&mut panel.batt_hist, batt.round() as u64);
                panel.status = Some(status);
                panel.error = None;
                panel.last_ok = Some(Instant::now());
            }
            Err(e) => panel.error = Some(e),
        }
    }

    /// Per-frame housekeeping: expire toasts.
    pub fn tick(&mut self) {
        if let Some(t) = &self.toast {
            if !t.alive() {
                self.toast = None;
            }
        }
    }

    fn toast_info(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::new(msg, ToastKind::Info));
    }

    /// Public info toast, e.g. the startup source banner.
    pub fn notify_info(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::new(msg, ToastKind::Info));
    }
    fn toast_ok(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::new(msg, ToastKind::Success));
    }
    fn toast_err(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::new(msg, ToastKind::Error));
    }

    fn goto(&mut self, v: View) {
        self.prev_view = self.view;
        self.view = v;
    }

    pub fn selected_panel(&self) -> Option<&UpsPanel> {
        self.upses.get(self.selected)
    }

    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        // Editor and client-gen capture text input when editing.
        if self.view == View::Editor {
            if let Some(ed) = &self.editor {
                if ed.editing {
                    self.editor_handle_text(code);
                    return;
                }
            }
        }
        if self.view == View::ClientGen {
            if let Some(cg) = &self.clientgen {
                if cg.editing {
                    self.clientgen_handle_text(code);
                    return;
                }
            }
        }

        match self.view {
            View::Dashboard => self.dashboard_key(code, mods),
            View::Detail => self.detail_key(code),
            View::Editor => self.editor_key(code),
            View::Services => self.services_key(code),
            View::ClientGen => self.clientgen_key(code),
            View::Events => self.events_key(code),
            View::Help => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.view = self.prev_view;
                }
            }
        }
    }

    // ---- dashboard ----
    fn dashboard_key(&mut self, code: KeyCode, _mods: KeyModifiers) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('b') => self.basic = !self.basic,
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                let msg = if self.paused { "sampling paused" } else { "sampling resumed" };
                self.toast_info(msg);
            }
            KeyCode::Char('?') | KeyCode::Char('h') => self.goto(View::Help),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                self.detail_scroll = 0;
                self.goto(View::Detail);
            }
            KeyCode::Char('e') => {
                self.events = crate::events::load_tail(500);
                self.events_scroll = 0;
                self.goto(View::Events);
            }
            KeyCode::Char('c') => self.open_editor(),
            KeyCode::Char('s') => self.open_services(),
            KeyCode::Char('g') => self.open_clientgen(),
            _ => {}
        }
    }

    fn move_sel(&mut self, delta: i32) {
        if self.upses.is_empty() {
            return;
        }
        let n = self.upses.len() as i32;
        self.selected = (((self.selected as i32 + delta) % n + n) % n) as usize;
    }

    // ---- detail ----
    fn detail_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left => {
                self.view = View::Dashboard;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            KeyCode::Char('c') => self.open_editor(),
            _ => {}
        }
    }

    // ---- events ----
    fn events_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => self.view = self.prev_view,
            KeyCode::Char('j') | KeyCode::Down => {
                self.events_scroll = self.events_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.events_scroll = self.events_scroll.saturating_sub(1);
            }
            KeyCode::Char('r') => {
                self.events = crate::events::load_tail(500);
                self.toast_info("events reloaded");
            }
            _ => {}
        }
    }

    // ---- editor ----
    fn open_editor(&mut self) {
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

    fn editor_key(&mut self, code: KeyCode) {
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

    fn editor_handle_text(&mut self, code: KeyCode) {
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

    // ---- services ----
    fn open_services(&mut self) {
        let instances = service::discover();
        if instances.is_empty() {
            self.toast_err("no apcupsd configs found in /etc/apcupsd");
            return;
        }
        self.services = Some(ServicesState { instances, cursor: 0, confirm: None });
        self.last_discovery = Some(Instant::now());
        self.goto(View::Services);
    }

    fn services_key(&mut self, code: KeyCode) {
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

    // ---- client gen ----
    fn open_clientgen(&mut self) {
        // One tab per monitored unit, each seeded from its on-disk config
        // when available (master address guessed from the unit's NIS host).
        let mut tabs = Vec::new();
        for panel in &self.upses {
            let mut params = crate::clientgen::ClientParams::default();
            let path = std::path::Path::new(service::CONF_DIR)
                .join(format!("{}.conf", panel.name));
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let cf = ConfigFile::parse(&raw);
                let port = cf.get("NISPORT").unwrap_or("3551").to_string();
                let host = panel.addr.split(':').next().unwrap_or("127.0.0.1");
                params = crate::clientgen::suggest_from_master(&cf, &format!("{host}:{port}"));
            } else {
                params.ups_name = panel.name.clone();
                params.master_addr = panel.addr.clone();
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

    fn clientgen_key(&mut self, code: KeyCode) {
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

    fn clientgen_handle_text(&mut self, code: KeyCode) {
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

fn push_hist(buf: &mut VecDeque<u64>, v: u64) {
    if buf.len() == HISTORY {
        buf.pop_front();
    }
    buf.push_back(v);
}

/// Test-support: build an app with synthetic panels and an injected status,
/// so UI rendering can be exercised without a live daemon. Behind cfg(test)
/// only via the public-but-hidden marker; used by integration tests.
#[doc(hidden)]
impl App {
    pub fn test_fixture(basic: bool) -> Self {
        use crate::nis::UpsStatus;
        let refs = [
            UpsRef { name: "rack-main".into(), addr: "127.0.0.1:3551".into() },
            UpsRef { name: "rack-aux".into(), addr: "127.0.0.1:3552".into() },
        ];
        let mut app = App::new(&refs, basic);
        let mut fields = std::collections::HashMap::new();
        for (k, v) in [
            ("STATUS", "ONLINE"), ("LINEV", "121.5 Volts"), ("LOADPCT", "42.0 Percent"),
            ("BCHARGE", "93.0 Percent"), ("TIMELEFT", "22.0 Minutes"), ("BATTV", "27.3 Volts"),
            ("NOMPOWER", "900 Watts"), ("MODEL", "Smart-UPS 1500"), ("NUMXFERS", "3"),
            ("SERIALNO", "ABC123"), ("HOSTNAME", "minipc"), ("VERSION", "3.14"),
        ] {
            fields.insert(k.to_string(), v.to_string());
        }
        app.upses[0].status = Some(UpsStatus { fields: fields.clone() });
        for i in 0..50u64 {
            push_hist(&mut app.upses[0].load_hist, 30 + i % 20);
            push_hist(&mut app.upses[0].batt_hist, 90 + i % 10);
        }
        // second unit on battery
        let mut f2 = fields.clone();
        f2.insert("STATUS".into(), "ONBATT".into());
        f2.insert("LINEV".into(), "0.0 Volts".into());
        app.upses[1].status = Some(UpsStatus { fields: f2 });
        app
    }

    pub fn test_set_view(&mut self, v: View) {
        self.view = v;
    }

    pub fn test_pause(&mut self) {
        self.paused = true;
    }

    /// Read-only access to editor state for tests.
    pub fn editor_ref(&self) -> Option<&EditorState> {
        self.editor.as_ref()
    }

    /// Populate the editor view from in-memory config texts (no disk access),
    /// one tab per (name, text) pair.
    pub fn test_open_editor_tabs(&mut self, confs: &[(&str, &str)]) {
        let tabs = confs
            .iter()
            .map(|(name, text)| {
                let original = ConfigFile::parse(text);
                let working = original.clone();
                let fields = build_fields(&working);
                let findings = config::validate(&working);
                EditorTab {
                    instance: name.to_string(),
                    path: std::path::PathBuf::from(format!("/etc/apcupsd/{name}.conf")),
                    original,
                    working,
                    fields,
                    cursor: 0,
                    findings,
                }
            })
            .collect();
        self.editor = Some(EditorState {
            tabs,
            active: 0,
            editing: false,
            edit_buffer: String::new(),
            show_diff: false,
        });
        self.view = View::Editor;
    }

    /// Single-tab convenience used by older tests.
    pub fn test_open_editor(&mut self, conf_text: &str) {
        self.test_open_editor_tabs(&[("rack-main", conf_text)]);
    }

    /// Toggle the editor diff overlay (test support).
    pub fn test_editor_show_diff(&mut self) {
        if let Some(ed) = &mut self.editor {
            // make a change so the diff is non-empty
            ed.tab_mut().working.set("BATTERYLEVEL", "20");
            ed.show_diff = true;
        }
    }

    /// Populate the services view with synthetic instances.
    pub fn test_open_services(&mut self) {
        use crate::service::{ActiveState, Instance};
        let instances = vec![
            Instance {
                name: "rack-main".into(),
                conf_path: "/etc/apcupsd/rack-main.conf".into(),
                active: ActiveState::Active,
                enabled: true,
                nis_addr: Some("127.0.0.1:3551".into()),
            },
            Instance {
                name: "rack-aux".into(),
                conf_path: "/etc/apcupsd/rack-aux.conf".into(),
                active: ActiveState::Failed,
                enabled: false,
                nis_addr: Some("127.0.0.1:3552".into()),
            },
        ];
        self.services = Some(ServicesState { instances, cursor: 0, confirm: None });
        self.view = View::Services;
    }

    /// Show the services stop-confirmation modal (test support).
    pub fn test_services_confirm_stop(&mut self) {
        if let Some(sv) = &mut self.services {
            sv.confirm = Some((service::ServiceAction::Stop, "rack-aux".into()));
        }
    }

    /// Populate the client-gen view (two tabs).
    pub fn test_open_clientgen(&mut self) {
        let tabs = ["rack-main", "rack-aux"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let params = crate::clientgen::ClientParams {
                    master_addr: format!("192.168.1.10:{}", 3551 + i),
                    ups_name: name.to_string(),
                    ..Default::default()
                };
                let preview = crate::clientgen::render_conf(&params);
                ClientGenTab {
                    instance: name.to_string(),
                    params,
                    preview,
                    saved_path: None,
                }
            })
            .collect();
        self.clientgen = Some(ClientGenState {
            tabs,
            active: 0,
            cursor: 0,
            editing: false,
            edit_buffer: String::new(),
        });
        self.view = View::ClientGen;
    }
}

/// Load one editor tab from /etc/apcupsd/<name>.conf, if readable.
fn load_editor_tab(name: &str) -> Option<EditorTab> {
    let path = std::path::Path::new(service::CONF_DIR).join(format!("{name}.conf"));
    let raw = std::fs::read_to_string(&path).ok()?;
    let original = ConfigFile::parse(&raw);
    let working = original.clone();
    let fields = build_fields(&working);
    let findings = config::validate(&working);
    Some(EditorTab {
        instance: name.to_string(),
        path,
        original,
        working,
        fields,
        cursor: 0,
        findings,
    })
}

/// Build editor fields from the file: every schema directive, ordered by
/// group then catalog order, marking which are present. Returns also the
/// list of non-schema directive keys we'll preserve verbatim.
fn build_fields(cf: &ConfigFile) -> Vec<EditField> {
    let mut fields = Vec::new();
    for group in config::Group::order() {
        for d in config::schema::catalog().iter().filter(|d| d.group == *group) {
            let present = cf.get(d.key).is_some();
            // Show all common directives; show uncommon only if present.
            if !present && !d.common {
                continue;
            }
            let value = cf.get(d.key).unwrap_or("").to_string();
            fields.push(EditField {
                key: d.key.to_string(),
                group: d.group,
                value,
                present,
                help: d.help.to_string(),
                kind: d.kind.clone(),
            });
        }
    }
    fields
}

fn clientgen_field_value(p: &crate::clientgen::ClientParams, idx: usize) -> String {
    match idx {
        0 => p.master_addr.clone(),
        1 => p.ups_name.clone(),
        2 => p.battery_level.to_string(),
        3 => p.minutes.to_string(),
        4 => p.polltime.to_string(),
        _ => String::new(),
    }
}

fn clientgen_set_field(p: &mut crate::clientgen::ClientParams, idx: usize, val: &str) {
    match idx {
        0 => p.master_addr = val.trim().to_string(),
        1 => p.ups_name = val.trim().to_string(),
        2 => {
            if let Ok(n) = val.trim().parse() {
                p.battery_level = n;
            }
        }
        3 => {
            if let Ok(n) = val.trim().parse() {
                p.minutes = n;
            }
        }
        4 => {
            if let Ok(n) = val.trim().parse() {
                p.polltime = n;
            }
        }
        _ => {}
    }
}

pub fn clientgen_field_label(idx: usize) -> &'static str {
    match idx {
        0 => "master address (host:port)",
        1 => "UPS name",
        2 => "client BATTERYLEVEL (%)",
        3 => "client MINUTES",
        4 => "POLLTIME (s)",
        _ => "",
    }
}

fn dirs_output() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("apctui-client-bundles")
}
