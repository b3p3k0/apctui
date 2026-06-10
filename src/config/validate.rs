// SPDX-License-Identifier: GPL-3.0-or-later
//! Validation for apcupsd configs. apcupsd ships no `--check`, so this is the
//! only guard between the user and a daemon that won't start. We separate
//! hard errors (daemon will fail or misbehave) from warnings (legal but
//! probably not what you want).

use super::parser::ConfigFile;
use super::schema::{self, Kind};

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub key: Option<String>,
    pub message: String,
}

impl Finding {
    fn error(key: Option<&str>, msg: impl Into<String>) -> Self {
        Finding { severity: Severity::Error, key: key.map(String::from), message: msg.into() }
    }
    fn warn(key: Option<&str>, msg: impl Into<String>) -> Self {
        Finding { severity: Severity::Warning, key: key.map(String::from), message: msg.into() }
    }
}

/// Validate a single directive's value against its schema entry.
/// Returns None if valid, or a Finding describing the problem.
pub fn check_value(key: &str, value: &str) -> Option<Finding> {
    let d = schema::lookup(key)?;
    match &d.kind {
        Kind::Text => None,
        Kind::Bool => match value.to_ascii_lowercase().as_str() {
            "on" | "off" => None,
            _ => Some(Finding::error(Some(key), format!("{key} must be 'on' or 'off', got '{value}'"))),
        },
        Kind::Enum(opts) => {
            if opts.iter().any(|o| o.eq_ignore_ascii_case(value)) {
                None
            } else {
                Some(Finding::error(
                    Some(key),
                    format!("{key} must be one of: {}", opts.join(", ")),
                ))
            }
        }
        Kind::Int { min, max, .. } => match value.trim().parse::<i64>() {
            Ok(n) if n >= *min && n <= *max => None,
            Ok(n) => Some(Finding::error(
                Some(key),
                format!("{key}={n} out of range [{min}, {max}]"),
            )),
            Err(_) => Some(Finding::error(
                Some(key),
                format!("{key} must be an integer, got '{value}'"),
            )),
        },
    }
}

/// Full-file validation: per-value checks plus cross-directive rules.
pub fn validate(cf: &ConfigFile) -> Vec<Finding> {
    let mut findings = Vec::new();
    let directives = cf.directives();

    // per-value
    for (key, value) in &directives {
        if let Some(f) = check_value(key, value) {
            findings.push(f);
        }
    }

    let get = |k: &str| cf.get(k).map(str::trim).map(str::to_string);
    let upstype = get("UPSTYPE").unwrap_or_default().to_ascii_lowercase();
    let upscable = get("UPSCABLE").unwrap_or_default().to_ascii_lowercase();
    let device = get("DEVICE");

    // UPSTYPE present at all?
    if upstype.is_empty() {
        findings.push(Finding::warn(Some("UPSTYPE"), "UPSTYPE not set; apcupsd needs one"));
    }

    // DEVICE rules by type (man page: usb => blank, net/snmp/pcnet => host spec,
    // serial types => /dev path).
    match upstype.as_str() {
        "usb" => {
            if device.as_deref().map(|d| !d.is_empty()).unwrap_or(false) {
                findings.push(Finding::warn(
                    Some("DEVICE"),
                    "USB UPSTYPE usually wants DEVICE blank (apcupsd autodetects); a path is only for advanced setups",
                ));
            }
        }
        "net" => match device.as_deref() {
            Some(d) if d.contains(':') => {}
            _ => findings.push(Finding::error(
                Some("DEVICE"),
                "UPSTYPE net requires DEVICE host:port",
            )),
        },
        "snmp" => {
            if device.as_deref().map(|d| d.matches(':').count() < 3).unwrap_or(true) {
                findings.push(Finding::error(
                    Some("DEVICE"),
                    "UPSTYPE snmp requires DEVICE host:port:vendor:community",
                ));
            }
        }
        "apcsmart" | "dumb" | "modbus" => {
            if upscable != "usb"
                && device.as_deref().map(str::is_empty).unwrap_or(true)
            {
                findings.push(Finding::warn(
                    Some("DEVICE"),
                    format!("UPSTYPE {upstype} usually needs a serial DEVICE like /dev/ttyS0"),
                ));
            }
        }
        _ => {}
    }

    // DEVICE is ignored for UPSCABLE ether (man page).
    if upscable == "ether"
        && device.as_deref().map(|d| !d.is_empty()).unwrap_or(false)
    {
        findings.push(Finding::warn(
            Some("DEVICE"),
            "DEVICE is ignored when UPSCABLE is ether",
        ));
    }

    // Shutdown policy sanity: all three disabled => UPS will never trigger
    // an automatic shutdown.
    let batt = get("BATTERYLEVEL").and_then(|v| v.parse::<i64>().ok());
    let mins = get("MINUTES").and_then(|v| v.parse::<i64>().ok());
    let tmo = get("TIMEOUT").and_then(|v| v.parse::<i64>().ok());
    let batt_off = matches!(batt, Some(-1)) || batt.is_none();
    let mins_off = matches!(mins, Some(-1)) || mins.is_none();
    let tmo_off = matches!(tmo, Some(0)) || tmo.is_none();
    if batt_off && mins_off && tmo_off {
        findings.push(Finding::warn(
            None,
            "no shutdown trigger set (BATTERYLEVEL/MINUTES/TIMEOUT all off); host will not auto-shutdown on battery",
        ));
    }

    // NIS coherence
    let netserver = get("NETSERVER").unwrap_or_else(|| "on".into()).to_ascii_lowercase();
    if netserver == "on" {
        if get("NISPORT").is_none() {
            findings.push(Finding::warn(Some("NISPORT"), "NETSERVER on but NISPORT unset (defaults to 3551)"));
        }
        if let Some(ip) = get("NISIP") {
            if ip == "127.0.0.1" {
                findings.push(Finding::warn(
                    Some("NISIP"),
                    "NISIP 127.0.0.1 allows local connections only; net clients on the LAN cannot reach this UPS",
                ));
            }
        }
    }

    // Duplicate directives (apcupsd uses the last; usually a mistake).
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for (key, _) in &directives {
        *seen.entry(key.to_ascii_uppercase()).or_default() += 1;
    }
    for (key, count) in seen {
        if count > 1 {
            findings.push(Finding::warn(
                Some(&key),
                format!("{key} appears {count} times; apcupsd uses the last occurrence"),
            ));
        }
    }

    findings
}

pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_validation() {
        assert!(check_value("UPSTYPE", "usb").is_none());
        assert!(check_value("UPSTYPE", "bogus").is_some());
    }

    #[test]
    fn int_range_and_sentinel() {
        assert!(check_value("BATTERYLEVEL", "10").is_none());
        assert!(check_value("BATTERYLEVEL", "-1").is_none()); // sentinel ok (in range)
        assert!(check_value("BATTERYLEVEL", "150").is_some());
        assert!(check_value("NISPORT", "70000").is_some());
        assert!(check_value("NISPORT", "abc").is_some());
    }

    #[test]
    fn bool_validation() {
        assert!(check_value("NETSERVER", "on").is_none());
        assert!(check_value("NETSERVER", "ON").is_none());
        assert!(check_value("NETSERVER", "yes").is_some());
    }

    #[test]
    fn usb_with_device_warns() {
        let cf = ConfigFile::parse("UPSTYPE usb\nUPSCABLE usb\nDEVICE /dev/ttyS0\n");
        let f = validate(&cf);
        assert!(f.iter().any(|x| x.severity == Severity::Warning
            && x.key.as_deref() == Some("DEVICE")));
    }

    #[test]
    fn net_without_hostport_errors() {
        let cf = ConfigFile::parse("UPSTYPE net\nDEVICE\n");
        let f = validate(&cf);
        assert!(has_errors(&f));
    }

    #[test]
    fn net_with_hostport_ok() {
        let cf = ConfigFile::parse("UPSTYPE net\nUPSCABLE ether\nDEVICE 10.0.0.5:3551\n");
        let f = validate(&cf);
        assert!(!has_errors(&f));
    }

    #[test]
    fn no_shutdown_trigger_warns() {
        let cf = ConfigFile::parse("UPSTYPE usb\nBATTERYLEVEL -1\nMINUTES -1\nTIMEOUT 0\n");
        let f = validate(&cf);
        assert!(f.iter().any(|x| x.message.contains("no shutdown trigger")));
    }

    #[test]
    fn valid_usb_config_clean() {
        let cf = ConfigFile::parse(
            "UPSNAME rack-main\nUPSCABLE usb\nUPSTYPE usb\nDEVICE\nBATTERYLEVEL 10\nMINUTES 5\nTIMEOUT 0\nNETSERVER on\nNISIP 0.0.0.0\nNISPORT 3551\n",
        );
        let f = validate(&cf);
        assert!(!has_errors(&f), "unexpected errors: {f:?}");
    }

    #[test]
    fn duplicate_directive_warns() {
        let cf = ConfigFile::parse("MINUTES 5\nMINUTES 10\n");
        let f = validate(&cf);
        assert!(f.iter().any(|x| x.message.contains("appears 2 times")));
    }
}
