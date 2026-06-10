// SPDX-License-Identifier: GPL-3.0-or-later
//! Push notifications for UPS state transitions. Detection happens in
//! `App::apply` (pure, testable); this module owns delivery: a background
//! worker thread drains a channel and POSTs to Pushbullet, so a slow or
//! dead network never blocks the UI. Worker errors come back on a status
//! channel and surface as toasts.
//!
//! Testing hook: APCTUI_PUSHBULLET_URL overrides the API endpoint so the
//! suite can run against a local mock server. The sandbox this code is
//! developed in cannot reach api.pushbullet.com; the request shape follows
//! https://docs.pushbullet.com/#create-push (POST /v2/pushes, Access-Token
//! header, {"type":"note","title":...,"body":...}).

use crate::options::Notifications;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

const PUSHBULLET_URL: &str = "https://api.pushbullet.com/v2/pushes";

fn endpoint() -> String {
    std::env::var("APCTUI_PUSHBULLET_URL").unwrap_or_else(|_| PUSHBULLET_URL.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    OnBattery,
    OnLine,
    CommLost,
    CommRestored,
    Test,
}

impl EventKind {
    pub fn title(&self, unit: &str) -> String {
        match self {
            EventKind::OnBattery => format!("{unit}: ON BATTERY"),
            EventKind::OnLine => format!("{unit}: back on line power"),
            EventKind::CommLost => format!("{unit}: communication lost"),
            EventKind::CommRestored => format!("{unit}: communication restored"),
            EventKind::Test => "apctui test notification".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotifyEvent {
    pub unit: String,
    pub kind: EventKind,
    /// Extra context for the notification body (load/runtime, error text...).
    pub detail: String,
}

/// Per-(unit, kind) rate limiter. Pure; time is injected for testability.
pub struct CooldownGate {
    window: Duration,
    last: HashMap<(String, EventKind), Instant>,
}

impl CooldownGate {
    pub fn new(window: Duration) -> Self {
        CooldownGate { window, last: HashMap::new() }
    }

    /// True if the event may pass; records it as sent at `now`.
    pub fn admit_at(&mut self, ev: &NotifyEvent, now: Instant) -> bool {
        // Test pushes always go through; the user asked for one explicitly.
        if ev.kind == EventKind::Test {
            return true;
        }
        let key = (ev.unit.clone(), ev.kind);
        match self.last.get(&key) {
            Some(t) if now.duration_since(*t) < self.window => false,
            _ => {
                self.last.insert(key, now);
                true
            }
        }
    }

    pub fn admit(&mut self, ev: &NotifyEvent) -> bool {
        self.admit_at(ev, Instant::now())
    }
}

/// Outcome reports from the worker, for toasting.
#[derive(Debug, Clone)]
pub enum NotifyStatus {
    Sent(EventKind),
    Failed(String),
}

pub struct Notifier {
    tx: Option<Sender<NotifyEvent>>,
    status_rx: Option<Receiver<NotifyStatus>>,
}

impl Notifier {
    /// No-op notifier: used when notifications are disabled and in tests.
    pub fn disabled() -> Self {
        Notifier { tx: None, status_rx: None }
    }

    /// Spawn the delivery worker. Returns a disabled notifier if the config
    /// can't deliver anything (disabled, or no token).
    pub fn spawn(opts: &Notifications) -> Self {
        if !opts.enabled || opts.pushbullet_token.is_empty() {
            return Notifier::disabled();
        }
        let token = opts.pushbullet_token.clone();
        let url = endpoint();
        let cooldown = Duration::from_secs(opts.cooldown_secs);
        let (tx, rx) = mpsc::channel::<NotifyEvent>();
        let (status_tx, status_rx) = mpsc::channel::<NotifyStatus>();
        std::thread::Builder::new()
            .name("apctui-notify".into())
            .spawn(move || worker(rx, status_tx, token, url, cooldown))
            .expect("spawning notifier thread");
        Notifier { tx: Some(tx), status_rx: Some(status_rx) }
    }

    pub fn is_active(&self) -> bool {
        self.tx.is_some()
    }

    pub fn send(&self, ev: NotifyEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ev); // worker gone == nothing to do
        }
    }

    /// Non-blocking: collect any delivery outcomes since last poll.
    pub fn poll_status(&self) -> Vec<NotifyStatus> {
        let mut out = Vec::new();
        if let Some(rx) = &self.status_rx {
            loop {
                match rx.try_recv() {
                    Ok(s) => out.push(s),
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
        }
        out
    }
}

fn worker(
    rx: Receiver<NotifyEvent>,
    status: Sender<NotifyStatus>,
    token: String,
    url: String,
    cooldown: Duration,
) {
    let mut gate = CooldownGate::new(cooldown);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build();
    while let Ok(ev) = rx.recv() {
        if !gate.admit(&ev) {
            continue;
        }
        let title = ev.kind.title(&ev.unit);
        let body = if ev.detail.is_empty() { title.clone() } else { ev.detail.clone() };
        let resp = agent
            .post(&url)
            .set("Access-Token", &token)
            .send_json(ureq::json!({
                "type": "note",
                "title": title,
                "body": body,
            }));
        let report = match resp {
            Ok(_) => NotifyStatus::Sent(ev.kind),
            Err(ureq::Error::Status(code, _)) => {
                NotifyStatus::Failed(format!("pushbullet: HTTP {code}"))
            }
            Err(e) => {
                let first = e.to_string();
                let first = first.lines().next().unwrap_or("send failed");
                NotifyStatus::Failed(format!("pushbullet: {first}"))
            }
        };
        let _ = status.send(report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(unit: &str, kind: EventKind) -> NotifyEvent {
        NotifyEvent { unit: unit.into(), kind, detail: String::new() }
    }

    #[test]
    fn cooldown_suppresses_repeats_per_unit_and_kind() {
        let mut g = CooldownGate::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(g.admit_at(&ev("apc0", EventKind::OnBattery), t0));
        // same unit+kind inside window: suppressed
        assert!(!g.admit_at(&ev("apc0", EventKind::OnBattery), t0 + Duration::from_secs(30)));
        // different kind, same unit: passes
        assert!(g.admit_at(&ev("apc0", EventKind::OnLine), t0 + Duration::from_secs(30)));
        // different unit, same kind: passes
        assert!(g.admit_at(&ev("apc1", EventKind::OnBattery), t0 + Duration::from_secs(30)));
        // window expired: passes again
        assert!(g.admit_at(&ev("apc0", EventKind::OnBattery), t0 + Duration::from_secs(61)));
    }

    #[test]
    fn test_pushes_bypass_cooldown() {
        let mut g = CooldownGate::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(g.admit_at(&ev("", EventKind::Test), t0));
        assert!(g.admit_at(&ev("", EventKind::Test), t0 + Duration::from_secs(1)));
    }

    #[test]
    fn disabled_notifier_is_inert() {
        let n = Notifier::disabled();
        assert!(!n.is_active());
        n.send(ev("apc0", EventKind::OnBattery)); // must not panic
        assert!(n.poll_status().is_empty());
    }

    #[test]
    fn spawn_requires_enabled_and_token() {
        let mut o = crate::options::Notifications::default();
        o.enabled = true; // no token
        assert!(!Notifier::spawn(&o).is_active());
        o.pushbullet_token = "o.x".into();
        o.enabled = false;
        assert!(!Notifier::spawn(&o).is_active());
    }

    /// Full delivery path against a local one-shot HTTP server: verifies the
    /// request line, Access-Token header, and JSON body shape.
    #[test]
    fn worker_posts_pushbullet_shaped_request() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let mut req = String::new();
            // read until we have headers + body (Content-Length is small)
            loop {
                let n = sock.read(&mut buf).unwrap();
                req.push_str(&String::from_utf8_lossy(&buf[..n]));
                if let Some(hdr_end) = req.find("\r\n\r\n") {
                    let cl: usize = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if req.len() >= hdr_end + 4 + cl {
                        break;
                    }
                }
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
            req
        });

        std::env::set_var("APCTUI_PUSHBULLET_URL", format!("http://127.0.0.1:{port}/v2/pushes"));
        let opts = crate::options::Notifications {
            enabled: true,
            pushbullet_token: "o.sekrit".into(),
            cooldown_secs: 0,
            ..Default::default()
        };
        let n = Notifier::spawn(&opts);
        std::env::remove_var("APCTUI_PUSHBULLET_URL");
        assert!(n.is_active());
        n.send(NotifyEvent {
            unit: "rack-main".into(),
            kind: EventKind::OnBattery,
            detail: "load 48%, runtime 22m".into(),
        });

        let req = server.join().unwrap();
        assert!(req.starts_with("POST /v2/pushes"), "request line: {}", req.lines().next().unwrap_or(""));
        assert!(req.contains("Access-Token: o.sekrit"), "missing token header");
        assert!(req.contains("\"type\":\"note\""));
        assert!(req.contains("rack-main: ON BATTERY"));
        assert!(req.contains("load 48%, runtime 22m"));

        // worker reported success
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let st = n.poll_status();
            if st.iter().any(|s| matches!(s, NotifyStatus::Sent(EventKind::OnBattery))) {
                break;
            }
            assert!(Instant::now() < deadline, "no Sent status within 5s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
