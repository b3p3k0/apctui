// SPDX-License-Identifier: GPL-3.0-or-later
//! UPS instance registry. Sources, in priority order:
//!  1. `--ups NAME=HOST:PORT` CLI flags (repeatable)
//!  2. config file (`--config`, else ~/.config/apctui/config.toml)
//!  3. fallback: single local UPS on 127.0.0.1:3551
//!
//! Designed for N units (built and tested with 4 in mind).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct UpsRef {
    pub name: String,
    pub addr: String,
}

/// Where the UPS list came from — surfaced in the UI at startup so a
/// misconfigured setup is immediately visible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    CliFlags,
    ConfigFile,
    UserConfig,
    Discovered,
    /// Discovered local units unioned with config `[[ups]]` entries.
    Merged { local: usize, config: usize },
    Fallback,
}

impl Source {
    pub fn describe(self, n: usize) -> String {
        let unit = if n == 1 { "unit" } else { "units" };
        match self {
            Source::CliFlags => format!("{n} {unit} from --ups flags"),
            Source::ConfigFile => format!("{n} {unit} from --config file"),
            Source::UserConfig => format!("{n} {unit} from ~/.config/apctui/config.toml"),
            Source::Discovered => format!("discovered {n} {unit} in /etc/apcupsd"),
            Source::Merged { local, config } => {
                format!("{n} units: {local} local + {config} configured")
            }
            Source::Fallback => "no config found; trying 127.0.0.1:3551".to_string(),
        }
    }
}

#[derive(Deserialize)]
struct FileConfig {
    #[serde(default)]
    ups: Vec<FileUps>,
    #[serde(default)]
    discovery: DiscoveryOpts,
}

#[derive(Deserialize, Default)]
struct DiscoveryOpts {
    /// Instance names to hide from auto-discovery (e.g. units you only want
    /// visible in the services view).
    #[serde(default)]
    ignore: Vec<String>,
}

#[derive(Deserialize)]
struct FileUps {
    name: String,
    addr: String,
}

fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|base| base.join("apctui").join("config.toml"))
}

fn parse_cli_ups(spec: &str) -> Result<UpsRef> {
    let (name, addr) = spec
        .split_once('=')
        .with_context(|| format!("--ups expects NAME=HOST:PORT, got `{spec}`"))?;
    if !addr.contains(':') {
        bail!("--ups address must be HOST:PORT, got `{addr}`");
    }
    Ok(UpsRef {
        name: name.trim().to_string(),
        addr: addr.trim().to_string(),
    })
}

fn load_raw(path: &Path) -> Result<FileConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

fn load_file(path: &Path) -> Result<Vec<UpsRef>> {
    Ok(load_raw(path)?
        .ups
        .into_iter()
        .map(|u| UpsRef { name: u.name, addr: u.addr })
        .collect())
}

/// The discovery ignore list from the user config, if that file exists.
fn discovery_ignore() -> Vec<String> {
    default_config_path()
        .filter(|p| p.exists())
        .and_then(|p| load_raw(&p).ok())
        .map(|c| c.discovery.ignore)
        .unwrap_or_default()
}

/// `[[ups]]` entries from the user config file (empty if none / no file).
/// These are the units manageable in-app via the Units view.
pub fn configured_ups() -> Vec<UpsRef> {
    default_config_path()
        .filter(|p| p.exists())
        .and_then(|p| load_file(&p).ok())
        .unwrap_or_default()
}

/// Local apcupsd instances discovered by scanning /etc/apcupsd/*.conf, each
/// resolved to its NIS endpoint. Excludes the stock single-instance unit.
fn discover_locals() -> Vec<UpsRef> {
    crate::service::discover()
        .into_iter()
        .filter(|inst| inst.name != "apcupsd")
        .filter_map(|inst| inst.nis_addr.map(|addr| UpsRef { name: inst.name, addr }))
        .collect()
}

/// Union discovered local units with config `[[ups]]` entries. Discovered
/// units in the `ignore` list are dropped; on a name collision the config
/// entry wins (it's the explicit choice). Locals come first, then config.
fn merge(discovered: Vec<UpsRef>, configured: Vec<UpsRef>, ignore: &[String]) -> Vec<UpsRef> {
    use std::collections::HashSet;
    let config_names: HashSet<&str> = configured.iter().map(|u| u.name.as_str()).collect();
    let mut out: Vec<UpsRef> = discovered
        .into_iter()
        .filter(|d| !ignore.iter().any(|i| i == &d.name))
        .filter(|d| !config_names.contains(d.name.as_str()))
        .collect();
    out.extend(configured);
    out
}

pub fn resolve(cli_ups: &[String], cli_config: Option<&Path>) -> Result<Vec<UpsRef>> {
    resolve_with_source(cli_ups, cli_config).map(|(list, _)| list)
}

pub fn resolve_with_source(
    cli_ups: &[String],
    cli_config: Option<&Path>,
) -> Result<(Vec<UpsRef>, Source)> {
    // CLI flags are an explicit override and never merge.
    if !cli_ups.is_empty() {
        let list: Result<Vec<_>> = cli_ups.iter().map(|s| parse_cli_ups(s)).collect();
        return Ok((list?, Source::CliFlags));
    }

    // Configured [[ups]]: an explicit --config file must parse; the default
    // path is optional. These are merged with local discovery rather than
    // replacing it, so a hand-added LAN unit doesn't hide local ones.
    let (configured, explicit_config) = if let Some(path) = cli_config {
        (load_file(path)?, true)
    } else if let Some(path) = default_config_path().filter(|p| p.exists()) {
        (load_file(&path).unwrap_or_default(), false)
    } else {
        (Vec::new(), false)
    };

    let ignore = discovery_ignore();
    let merged = merge(discover_locals(), configured.clone(), &ignore);
    if !merged.is_empty() {
        let config = configured.len();
        let local = merged.len() - config;
        let source = match (local, config) {
            (0, _) if explicit_config => Source::ConfigFile,
            (0, _) => Source::UserConfig,
            (_, 0) => Source::Discovered,
            _ => Source::Merged { local, config },
        };
        return Ok((merged, source));
    }

    if explicit_config {
        bail!(
            "{} defines no [[ups]] entries and no local instances were found",
            cli_config.unwrap().display()
        );
    }
    Ok((
        vec![UpsRef {
            name: "local".to_string(),
            addr: "127.0.0.1:3551".to_string(),
        }],
        Source::Fallback,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_spec_parses() {
        let u = parse_cli_ups("rack-main=10.0.0.5:3551").unwrap();
        assert_eq!(u.name, "rack-main");
        assert_eq!(u.addr, "10.0.0.5:3551");
    }

    #[test]
    fn cli_spec_rejects_missing_port() {
        assert!(parse_cli_ups("rack-main=10.0.0.5").is_err());
    }

    fn r(name: &str, addr: &str) -> UpsRef {
        UpsRef { name: name.into(), addr: addr.into() }
    }
    fn names(v: &[UpsRef]) -> Vec<&str> {
        v.iter().map(|u| u.name.as_str()).collect()
    }

    #[test]
    fn merge_unions_locals_first_then_config() {
        let disc = vec![r("apc0", "127.0.0.1:3551"), r("apc1", "127.0.0.1:3552")];
        let cfg = vec![r("rack-lan", "192.168.1.50:3551")];
        let out = merge(disc, cfg, &[]);
        assert_eq!(names(&out), ["apc0", "apc1", "rack-lan"]);
    }

    #[test]
    fn merge_config_wins_on_name_collision() {
        let disc = vec![r("apc0", "127.0.0.1:3551"), r("apc1", "127.0.0.1:3552")];
        let cfg = vec![r("apc1", "10.0.0.9:3551")]; // same name, different addr
        let out = merge(disc, cfg, &[]);
        assert_eq!(names(&out), ["apc0", "apc1"]);
        // the surviving apc1 is the config one
        assert_eq!(out.iter().find(|u| u.name == "apc1").unwrap().addr, "10.0.0.9:3551");
    }

    #[test]
    fn merge_ignore_applies_to_discovered_only() {
        let disc = vec![r("apc0", "127.0.0.1:3551"), r("apc1", "127.0.0.1:3552")];
        let cfg = vec![r("rack-lan", "192.168.1.50:3551")];
        let out = merge(disc, cfg, &["apc0".to_string()]);
        assert_eq!(names(&out), ["apc1", "rack-lan"]);
    }

    #[test]
    fn merge_empty_config_is_discovery() {
        let disc = vec![r("apc0", "127.0.0.1:3551")];
        assert_eq!(names(&merge(disc.clone(), vec![], &[])), ["apc0"]);
    }

    #[test]
    fn toml_parses() {
        let cfg: FileConfig = toml::from_str(
            r#"
            [[ups]]
            name = "rack-main"
            addr = "127.0.0.1:3551"
            [[ups]]
            name = "rack-aux"
            addr = "127.0.0.1:3552"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.ups.len(), 2);
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    // resolve() falls through to /etc/apcupsd discovery only when no flags or
    // user config exist. We can't safely point CONF_DIR elsewhere here (it's a
    // const in the service module), so this test just asserts the fallback
    // ordering: explicit CLI flags always win over discovery.
    #[test]
    fn cli_flags_take_precedence_over_discovery() {
        let ups = vec!["only=10.0.0.9:3551".to_string()];
        let got = resolve(&ups, None).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "only");
        assert_eq!(got[0].addr, "10.0.0.9:3551");
    }
}
