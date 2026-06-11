// SPDX-License-Identifier: GPL-3.0-or-later
//! App-level options, persisted in the `[notifications]` section of
//! ~/.config/apctui/config.toml. Read with serde; written with toml_edit so
//! the user's [[ups]] entries, comments, and formatting survive untouched.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub provider: String,
    pub pushbullet_token: String,
    pub on_battery: bool,
    pub on_line: bool,
    pub comm_lost: bool,
    pub comm_restored: bool,
    pub cooldown_secs: u64,
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications {
            enabled: false,
            provider: "pushbullet".into(),
            pushbullet_token: String::new(),
            on_battery: true,
            on_line: true,
            comm_lost: true,
            comm_restored: true,
            cooldown_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct FileOptions {
    notifications: Notifications,
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|base| base.join("apctui").join("config.toml"))
}

pub fn load() -> Notifications {
    let Some(path) = config_path() else { return Notifications::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Notifications::default() };
    toml::from_str::<FileOptions>(&raw)
        .map(|f| f.notifications)
        .unwrap_or_default()
}

/// Write the [notifications] section, preserving everything else in the file
/// byte-for-byte. Creates the file (and directory) if absent. Sets mode 0600
/// when a token is present — it's a plaintext credential.
pub fn save(n: &Notifications) -> Result<PathBuf> {
    let path = config_path().context("cannot determine config path (no HOME)")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let t = &mut doc["notifications"];
    if t.is_none() {
        *t = toml_edit::Item::Table(toml_edit::Table::new());
    }
    t["enabled"] = toml_edit::value(n.enabled);
    t["provider"] = toml_edit::value(n.provider.as_str());
    t["pushbullet_token"] = toml_edit::value(n.pushbullet_token.as_str());
    t["on_battery"] = toml_edit::value(n.on_battery);
    t["on_line"] = toml_edit::value(n.on_line);
    t["comm_lost"] = toml_edit::value(n.comm_lost);
    t["comm_restored"] = toml_edit::value(n.comm_restored);
    t["cooldown_secs"] = toml_edit::value(n.cooldown_secs as i64);

    std::fs::write(&path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;
    if !n.pushbullet_token.is_empty() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_home<F: FnOnce()>(f: F) {
        let dir = std::env::temp_dir().join(format!("apctui-opt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        f();
        match old {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_load_roundtrip() {
        with_temp_home(|| {
            let mut n = Notifications::default();
            n.enabled = true;
            n.pushbullet_token = "o.test123".into();
            n.cooldown_secs = 120;
            save(&n).unwrap();
            let got = load();
            assert!(got.enabled);
            assert_eq!(got.pushbullet_token, "o.test123");
            assert_eq!(got.cooldown_secs, 120);
        });
    }

    #[test]
    fn save_preserves_existing_ups_entries_and_comments() {
        with_temp_home(|| {
            let path = config_path().unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let original = "# my units\n[[ups]]\nname = \"rack-main\"   # the big one\naddr = \"127.0.0.1:3551\"\n";
            std::fs::write(&path, original).unwrap();

            let n = Notifications { enabled: true, ..Default::default() };
            save(&n).unwrap();

            let after = std::fs::read_to_string(&path).unwrap();
            assert!(after.contains("# my units"), "comment lost: {after}");
            assert!(after.contains("# the big one"), "inline comment lost");
            assert!(after.contains("[[ups]]"));
            assert!(after.contains("[notifications]"));
        });
    }

    #[test]
    fn token_implies_0600() {
        with_temp_home(|| {
            use std::os::unix::fs::PermissionsExt;
            let n = Notifications {
                pushbullet_token: "o.secret".into(),
                ..Default::default()
            };
            let path = save(&n).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        });
    }
}
