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
    /// None until the first successful sample; transitions only after that.
    pub on_battery: Option<bool>,
    pub comm_fails: u8,
    pub comm_lost_notified: bool,
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
            on_battery: None,
            comm_fails: 0,
            comm_lost_notified: false,
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
    Options,
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

/// The app options form. Edits a working copy of Notifications; 's' persists
/// it and rebuilds the notifier so changes take effect immediately.
pub struct OptionsState {
    pub working: crate::options::Notifications,
    pub cursor: usize,
    pub editing: bool,
    pub edit_buffer: String,
    /// Unsaved-changes prompt is showing (raised by esc/q while dirty).
    pub confirm_close: bool,
}

pub const OPTIONS_FIELDS: usize = 9;

pub fn options_field_label(i: usize) -> &'static str {
    match i {
        0 => "notifications",
        1 => "provider",
        2 => "pushbullet token",
        3 => "notify: on battery",
        4 => "notify: back on line",
        5 => "notify: comm lost",
        6 => "notify: comm restored",
        7 => "cooldown (seconds)",
        8 => "send test notification",
        _ => "",
    }
}

pub fn options_field_help(i: usize) -> &'static str {
    match i {
        0 => "Master switch. Off means nothing is ever sent.",
        1 => "Delivery service. Pushbullet is the only one implemented.",
        2 => "Pushbullet access token, from pushbullet.com > Settings > Account. Stored plaintext in ~/.config/apctui/config.toml (chmod 600).",
        3 => "Push when a unit switches to battery power.",
        4 => "Push when line power returns.",
        5 => "Push after 3 consecutive failed polls of a unit.",
        6 => "Push when a lost unit starts answering again.",
        7 => "Minimum seconds between repeat pushes for the same unit and event.",
        8 => "Enter sends a test push with the current (unsaved) settings.",
        _ => "",
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
    pub options: Option<OptionsState>,
    pub detail_scroll: u16,
    last_discovery: Option<Instant>,
    pub notify_opts: crate::options::Notifications,
    notifier: crate::notify::Notifier,
    /// Short-lived notifier for the options-menu test push.
    test_notifier: Option<crate::notify::Notifier>,
    /// Throttle for standby lock-takeover attempts.
    standby_retry: Option<Instant>,
    pending_notifications: Vec<crate::notify::NotifyEvent>,
}

impl App {
    pub fn new(refs: &[UpsRef], basic: bool, notify_opts: crate::options::Notifications) -> Self {
        let notifier = crate::notify::Notifier::spawn(&notify_opts);
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
            options: None,
            detail_scroll: 0,
            last_discovery: None,
            notify_opts,
            notifier,
            test_notifier: None,
            standby_retry: None,
            pending_notifications: Vec::new(),
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

                // -------- notification detection (transitions only) --------
                // Comm loss comes in two flavors: the NIS socket failing
                // (daemon down / host unreachable, handled in Err below) and
                // a healthy daemon reporting STATUS COMMLOST (it lost the
                // UPS itself, e.g. USB unplugged). Both feed one counter.
                let daemon_commlost = status.status_text().contains("COMMLOST");
                if daemon_commlost {
                    panel.comm_fails = panel.comm_fails.saturating_add(1);
                    if panel.comm_fails == 3 && !panel.comm_lost_notified {
                        panel.comm_lost_notified = true;
                        self.pending_notifications.push(crate::notify::NotifyEvent {
                            unit: panel.name.clone(),
                            kind: crate::notify::EventKind::CommLost,
                            detail: "daemon reports COMMLOST (UPS link down)".to_string(),
                        });
                    }
                    // Don't run on-battery logic on a COMMLOST sample: the
                    // status fields are stale and the baseline must survive
                    // until real data returns.
                    panel.status = Some(status);
                    panel.error = None;
                    panel.last_ok = Some(Instant::now());
                    return;
                }
                if panel.comm_lost_notified {
                    self.pending_notifications.push(crate::notify::NotifyEvent {
                        unit: panel.name.clone(),
                        kind: crate::notify::EventKind::CommRestored,
                        detail: String::new(),
                    });
                }
                panel.comm_fails = 0;
                panel.comm_lost_notified = false;

                let now_onbatt = status.status_text().contains("ONBATT");
                if let Some(was) = panel.on_battery {
                    if !was && now_onbatt {
                        let runtime = status.num("TIMELEFT").unwrap_or(0.0);
                        self.pending_notifications.push(crate::notify::NotifyEvent {
                            unit: panel.name.clone(),
                            kind: crate::notify::EventKind::OnBattery,
                            detail: format!(
                                "load {:.0}%, est. runtime {:.0} min",
                                load, runtime
                            ),
                        });
                    } else if was && !now_onbatt {
                        self.pending_notifications.push(crate::notify::NotifyEvent {
                            unit: panel.name.clone(),
                            kind: crate::notify::EventKind::OnLine,
                            detail: format!("battery at {:.0}%", batt),
                        });
                    }
                }
                panel.on_battery = Some(now_onbatt);

                panel.status = Some(status);
                panel.error = None;
                panel.last_ok = Some(Instant::now());
            }
            Err(e) => {
                panel.comm_fails = panel.comm_fails.saturating_add(1);
                // 3 consecutive failures == lost, not a blip. Notify once.
                if panel.comm_fails == 3 && !panel.comm_lost_notified {
                    panel.comm_lost_notified = true;
                    self.pending_notifications.push(crate::notify::NotifyEvent {
                        unit: panel.name.clone(),
                        kind: crate::notify::EventKind::CommLost,
                        detail: e.clone(),
                    });
                }
                panel.error = Some(e);
            }
        }
    }

    /// Per-frame housekeeping: expire toasts, dispatch notifications.
    pub fn tick(&mut self) {
        if let Some(t) = &self.toast {
            if !t.alive() {
                self.toast = None;
            }
        }
        // Standby means another instance held notification duty. If it has
        // exited, the flock is free - take over so a closed primary doesn't
        // silently end notifications machine-wide.
        if self.notifier.state() == crate::notify::NotifierState::Standby
            && self.standby_retry.map_or(true, |t| t.elapsed() >= std::time::Duration::from_secs(10))
        {
            self.standby_retry = Some(Instant::now());
            let n = crate::notify::Notifier::spawn(&self.notify_opts);
            if n.state() == crate::notify::NotifierState::Active {
                self.notifier = n;
                self.toast_info("notification duty taken over (other instance closed)");
            }
        }

        // Dispatch detected transitions, filtered by the per-event toggles
        // in effect right now.
        for ev in self.pending_notifications.drain(..) {
            let wanted = match ev.kind {
                crate::notify::EventKind::OnBattery => self.notify_opts.on_battery,
                crate::notify::EventKind::OnLine => self.notify_opts.on_line,
                crate::notify::EventKind::CommLost => self.notify_opts.comm_lost,
                crate::notify::EventKind::CommRestored => self.notify_opts.comm_restored,
                crate::notify::EventKind::Test => true,
            };
            if wanted {
                self.notifier.send(ev);
            }
        }
        let mut test_done = false;
        if let Some(tn) = &self.test_notifier {
            for st in tn.poll_status() {
                test_done = true;
                match st {
                    crate::notify::NotifyStatus::Sent(_) => {
                        // The trap: the test action works even with the
                        // master switch off (so tokens can be verified before
                        // committing). Make that state impossible to miss.
                        if self.notify_opts.enabled {
                            self.toast_ok("test notification sent");
                        } else {
                            self.toast_info(
                                "test sent - but notifications are OFF; enable + save or real events won't send",
                            );
                        }
                    }
                    crate::notify::NotifyStatus::Failed(msg) => self.toast_err(msg),
                }
            }
        }
        if test_done {
            self.test_notifier = None;
        }
        for st in self.notifier.poll_status() {
            match st {
                // Real-event deliveries confirm on screen: ties detection ->
                // delivery visibly, so "no push arrived" is diagnosable.
                crate::notify::NotifyStatus::Sent(title) => {
                    self.toast_ok(format!("pushed: {title}"));
                }
                crate::notify::NotifyStatus::Failed(msg) => self.toast_err(msg),
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
        if self.view == View::Options {
            if let Some(op) = &self.options {
                if op.editing {
                    self.options_handle_text(code);
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
            View::Options => self.options_key(code),
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
            KeyCode::Char('o') => self.open_options(),
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
        let mut app = App::new(&refs, basic, crate::options::Notifications::default());
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

    /// Drain detected-but-undispatched notification events (test support).
    pub fn test_take_pending(&mut self) -> Vec<crate::notify::NotifyEvent> {
        std::mem::take(&mut self.pending_notifications)
    }

    /// Whether the live notifier is armed (enabled + token at last save/load).
    pub fn notifier_active(&self) -> bool {
        self.notifier.is_active()
    }

    /// Full notifier state, for the header indicator.
    pub fn notifier_state(&self) -> crate::notify::NotifierState {
        self.notifier.state()
    }

    /// Read-only options state access (test support).
    pub fn options_ref(&self) -> Option<&OptionsState> {
        self.options.as_ref()
    }

    /// Open the options view with given working settings (test support).
    pub fn test_open_options(&mut self, working: crate::options::Notifications) {
        self.options = Some(OptionsState {
            working,
            cursor: 0,
            editing: false,
            edit_buffer: String::new(),
            confirm_close: false,
        });
        self.view = View::Options;
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

impl App {
    // ---- options ----
    fn open_options(&mut self) {
        self.options = Some(OptionsState {
            working: self.notify_opts.clone(),
            cursor: 0,
            editing: false,
            edit_buffer: String::new(),
            confirm_close: false,
        });
        self.goto(View::Options);
    }

    fn options_key(&mut self, code: KeyCode) {
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

    fn options_handle_text(&mut self, code: KeyCode) {
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
                self.notifier = crate::notify::Notifier::spawn(&self.notify_opts);
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
