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

/// Normalize and validate a NIS endpoint. A bare host gets the default
/// :3551 port. Returns the normalized `HOST:PORT` or a user-facing error.
pub fn validate_addr(input: &str) -> std::result::Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("host is required".into());
    }
    let addr = if s.contains(':') { s.to_string() } else { format!("{s}:3551") };
    let (host, port) = addr.rsplit_once(':').unwrap(); // addr now always has ':'
    if host.is_empty() {
        return Err("host is required".into());
    }
    if port.parse::<u16>().is_err() {
        return Err(format!("invalid port `{port}` (expected 1-65535)"));
    }
    Ok(addr)
}

/// Append a `[[ups]]` entry to the config file, preserving everything else
/// byte-for-byte (comments, [notifications], sibling entries). Rejects a
/// duplicate name or a malformed address. Returns the file path.
pub fn add_ups(name: &str, addr: &str) -> Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("name is required");
    }
    let addr = validate_addr(addr).map_err(|e| anyhow::anyhow!(e))?;

    let path = config_path().context("cannot determine config path (no HOME)")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let item = &mut doc["ups"];
    if !item.is_array_of_tables() {
        *item = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let arr = item.as_array_of_tables_mut().expect("ups is an array-of-tables");
    if arr.iter().any(|t| t.get("name").and_then(|v| v.as_str()) == Some(name)) {
        anyhow::bail!("a unit named `{name}` already exists");
    }
    let mut tbl = toml_edit::Table::new();
    tbl["name"] = toml_edit::value(name);
    tbl["addr"] = toml_edit::value(addr.as_str());
    arr.push(tbl);

    std::fs::write(&path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Remove the `[[ups]]` entry with the given name. Returns whether one was
/// removed. Preserves everything else in the file.
pub fn remove_ups(name: &str) -> Result<bool> {
    let path = config_path().context("cannot determine config path (no HOME)")?;
    let Ok(raw) = std::fs::read_to_string(&path) else { return Ok(false) };
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let Some(arr) = doc["ups"].as_array_of_tables_mut() else { return Ok(false) };
    let Some(i) = arr
        .iter()
        .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
    else {
        return Ok(false);
    };
    arr.remove(i);
    std::fs::write(&path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    // XDG_CONFIG_HOME is process-global and config_path() reads it, so two
    // tests setting it in parallel clobber each other. Serialize the whole
    // set/use/restore window. Poison-tolerant: a panicking test must not
    // cascade-fail the rest by leaving the lock poisoned.
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    // Unique dir per call so a failed cleanup can't leak into the next test.
    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn with_temp_home<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "apctui-opt-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
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

    #[test]
    fn validate_addr_normalizes_and_rejects() {
        assert_eq!(validate_addr("192.168.1.5").unwrap(), "192.168.1.5:3551");
        assert_eq!(validate_addr(" 10.0.0.9:3552 ").unwrap(), "10.0.0.9:3552");
        assert!(validate_addr("").is_err());
        assert!(validate_addr("host:notaport").is_err());
        assert!(validate_addr(":3551").is_err());
    }

    #[test]
    fn add_ups_appends_and_preserves_siblings_and_comments() {
        with_temp_home(|| {
            let path = config_path().unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                "# header\n[notifications]\nenabled = true\n\n[[ups]]\nname = \"rack-main\"  # local\naddr = \"127.0.0.1:3551\"\n",
            )
            .unwrap();

            add_ups("rack-lan", "192.168.1.50:3551").unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(after.contains("# header"), "comment lost: {after}");
            assert!(after.contains("# local"), "inline comment lost");
            assert!(after.contains("rack-main"));
            assert!(after.contains("rack-lan"));
            assert!(after.contains("192.168.1.50:3551"));
            assert!(after.contains("[notifications]"));

            // a bare host gets the default port
            add_ups("shed", "192.168.1.71").unwrap();
            assert!(std::fs::read_to_string(&path).unwrap().contains("192.168.1.71:3551"));
        });
    }

    #[test]
    fn add_ups_rejects_duplicate_name() {
        with_temp_home(|| {
            add_ups("dup", "10.0.0.1:3551").unwrap();
            assert!(add_ups("dup", "10.0.0.2:3551").is_err());
        });
    }

    #[test]
    fn remove_ups_drops_entry_and_reports() {
        with_temp_home(|| {
            add_ups("a", "10.0.0.1:3551").unwrap();
            add_ups("b", "10.0.0.2:3551").unwrap();
            assert!(remove_ups("a").unwrap());
            assert!(!remove_ups("missing").unwrap());
            let path = config_path().unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(!after.contains("\"a\""), "removed entry still present: {after}");
            assert!(after.contains("\"b\""));
        });
    }
}
