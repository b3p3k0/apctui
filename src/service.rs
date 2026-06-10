// SPDX-License-Identifier: GPL-3.0-or-later
//! systemd service control for apcupsd instances, plus discovery of locally
//! configured instances from /etc/apcupsd/*.conf.
//!
//! Read-only status queries run unprivileged. State-changing actions
//! (start/stop/restart) are performed by the privileged `apctui apply`
//! helper, not here — this module only *describes* what needs doing and
//! queries current state.

use crate::config::ConfigFile;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CONF_DIR: &str = "/etc/apcupsd";

/// Conf files under CONF_DIR that are not daemon instances: hosts.conf and
/// multimon.conf configure apcupsd's legacy CGI monitoring tools.
const NON_INSTANCE_CONFS: &[&str] = &["hosts", "multimon"];

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveState {
    Active,
    Inactive,
    Failed,
    Unknown,
}

impl ActiveState {
    fn parse(s: &str) -> Self {
        match s.trim() {
            "active" => ActiveState::Active,
            "inactive" | "deactivating" => ActiveState::Inactive,
            "failed" => ActiveState::Failed,
            _ => ActiveState::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // conf_path retained for future "open by path"
pub struct Instance {
    /// Instance name (the part after @ in apcupsd@<name>).
    pub name: String,
    /// Path to its config file.
    pub conf_path: PathBuf,
    pub active: ActiveState,
    pub enabled: bool,
    /// NIS endpoint derived from the conf (NISIP/NISPORT), if NETSERVER on.
    pub nis_addr: Option<String>,
}

/// The systemd unit name for an instance.
pub fn unit_for(name: &str) -> String {
    format!("apcupsd@{name}.service")
}

fn systemctl_show(unit: &str, property: &str) -> Option<String> {
    let out = Command::new("systemctl")
        .args(["show", unit, "--property", property, "--value"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn query_state(name: &str) -> (ActiveState, bool) {
    let unit = unit_for(name);
    let active = systemctl_show(&unit, "ActiveState")
        .map(|s| ActiveState::parse(&s))
        .unwrap_or(ActiveState::Unknown);
    let enabled = systemctl_show(&unit, "UnitFileState")
        .map(|s| s == "enabled" || s == "enabled-runtime")
        .unwrap_or(false);
    (active, enabled)
}

fn nis_addr_from_conf(cf: &ConfigFile) -> Option<String> {
    let on = cf.get("NETSERVER").map(|v| v.eq_ignore_ascii_case("on")).unwrap_or(true);
    if !on {
        return None;
    }
    let ip = match cf.get("NISIP").map(str::trim) {
        Some("0.0.0.0") | None | Some("") => "127.0.0.1".to_string(),
        Some(other) => other.to_string(),
    };
    let port = cf.get("NISPORT").map(str::trim).unwrap_or("3551");
    Some(format!("{ip}:{port}"))
}

/// Discover instances by scanning CONF_DIR for `<name>.conf`. The stock
/// single-instance file is named `apcupsd.conf`; we treat its stem as an
/// instance too (it maps to the non-templated unit, but listing it is useful).
pub fn discover() -> Vec<Instance> {
    discover_in(Path::new(CONF_DIR))
}

fn discover_in(dir: &Path) -> Vec<Instance> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "conf").unwrap_or(false))
        .collect();
    paths.sort();

    for path in paths {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip backup/sample artifacts and the CGI-tool configs that the
        // apcupsd package ships alongside daemon configs.
        if stem.ends_with(".orig") || stem.ends_with("~") {
            continue;
        }
        if NON_INSTANCE_CONFS.contains(&stem.as_str()) {
            continue;
        }
        let name = if stem == "apcupsd" { "apcupsd".to_string() } else { stem };
        let cf = std::fs::read_to_string(&path)
            .ok()
            .map(|t| ConfigFile::parse(&t));
        let nis_addr = cf.as_ref().and_then(nis_addr_from_conf);
        let (active, enabled) = if name == "apcupsd" {
            // stock unit, not templated
            let active = systemctl_show("apcupsd.service", "ActiveState")
                .map(|s| ActiveState::parse(&s))
                .unwrap_or(ActiveState::Unknown);
            let enabled = systemctl_show("apcupsd.service", "UnitFileState")
                .map(|s| s.starts_with("enabled"))
                .unwrap_or(false);
            (active, enabled)
        } else {
            query_state(&name)
        };
        out.push(Instance { name, conf_path: path, active, enabled, nis_addr });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

impl ServiceAction {
    pub fn verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_naming() {
        assert_eq!(unit_for("rack-main"), "apcupsd@rack-main.service");
    }

    #[test]
    fn nis_addr_defaults_loopback_for_wildcard() {
        let cf = ConfigFile::parse("NETSERVER on\nNISIP 0.0.0.0\nNISPORT 3552\n");
        assert_eq!(nis_addr_from_conf(&cf).as_deref(), Some("127.0.0.1:3552"));
    }

    #[test]
    fn nis_addr_none_when_off() {
        let cf = ConfigFile::parse("NETSERVER off\n");
        assert_eq!(nis_addr_from_conf(&cf), None);
    }

    #[test]
    fn nis_addr_uses_explicit_ip() {
        let cf = ConfigFile::parse("NISIP 192.168.1.10\nNISPORT 3551\n");
        assert_eq!(nis_addr_from_conf(&cf).as_deref(), Some("192.168.1.10:3551"));
    }

    #[test]
    fn discover_skips_cgi_tool_confs() {
        let dir = std::env::temp_dir().join(format!("apctui-cgi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("apc0.conf"), "NISPORT 3551\n").unwrap();
        std::fs::write(dir.join("hosts.conf"), "MONITOR 127.0.0.1\n").unwrap();
        std::fs::write(dir.join("multimon.conf"), "HOSTS hosts.conf\n").unwrap();
        let names: Vec<_> = discover_in(&dir).into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["apc0"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_reads_conf_dir() {
        let dir = std::env::temp_dir().join(format!("apctui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rack-main.conf"), "NISPORT 3551\nNETSERVER on\n").unwrap();
        std::fs::write(dir.join("rack-aux.conf"), "NISPORT 3552\nNETSERVER on\n").unwrap();
        let found = discover_in(&dir);
        let names: Vec<_> = found.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"rack-main"));
        assert!(names.contains(&"rack-aux"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
