// SPDX-License-Identifier: GPL-3.0-or-later
//! Privilege escalation and the privileged apply path.
//!
//! Model: the TUI runs unprivileged. When the user confirms a change, the TUI
//! writes the proposed config to a temp file and re-invokes *itself* through
//! pkexec (falling back to sudo) as:
//!
//!   apctui apply --dest /etc/apcupsd/<name>.conf --src /tmp/xxx [--restart <name>]
//!                [--service <action>:<name>]
//!
//! The apply subcommand (run as root) re-validates, makes a timestamped
//! backup, writes atomically (temp + rename in the destination dir), and then
//! performs the requested systemctl action. One auth prompt; no long-lived
//! root; every step auditable.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{validate, ConfigFile};
use crate::service::{self, ServiceAction};

/// How we'll escalate. Detected once at point of use.
pub enum Escalator {
    Pkexec,
    Sudo,
    /// Already root — run directly.
    Direct,
}

pub fn detect_escalator() -> Escalator {
    // Already root?
    if is_root() {
        return Escalator::Direct;
    }
    if which("pkexec").is_some() {
        Escalator::Pkexec
    } else {
        Escalator::Sudo
    }
}

fn is_root() -> bool {
    // Avoid a libc dep: read /proc/self/status Uid line.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        })
        .unwrap_or(false)
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(bin))
        .find(|p| p.is_file())
}

fn self_exe() -> Result<PathBuf> {
    std::env::current_exe().context("locating own executable")
}

/// A planned change the TUI hands to the privileged helper.
pub struct ApplyPlan {
    pub dest: PathBuf,
    pub new_contents: String,
    /// Restart this instance after writing (the common case).
    pub restart: Option<String>,
    /// Or perform some other service action.
    pub service: Option<(ServiceAction, String)>,
}

/// Build the argv (after the escalator) for an apply plan, writing the source
/// temp file as a side effect. Returns (temp_path, args).
fn stage(plan: &ApplyPlan) -> Result<(PathBuf, Vec<String>)> {
    let exe = self_exe()?;
    let tmp = std::env::temp_dir().join(format!(
        "apctui-apply-{}-{}.conf",
        std::process::id(),
        plan.dest.file_stem().and_then(|s| s.to_str()).unwrap_or("conf")
    ));
    std::fs::write(&tmp, &plan.new_contents)
        .with_context(|| format!("writing staging file {}", tmp.display()))?;

    let mut args = vec![
        exe.to_string_lossy().into_owned(),
        "apply".to_string(),
        "--dest".to_string(),
        plan.dest.to_string_lossy().into_owned(),
        "--src".to_string(),
        tmp.to_string_lossy().into_owned(),
    ];
    if let Some(name) = &plan.restart {
        args.push("--restart".to_string());
        args.push(name.clone());
    }
    if let Some((action, name)) = &plan.service {
        args.push("--service".to_string());
        args.push(format!("{}:{}", action.verb(), name));
    }
    Ok((tmp, args))
}

/// Run an apply plan with escalation. Blocks until the helper exits.
/// Returns the helper's stdout/stderr on failure for display.
pub fn run_apply(plan: &ApplyPlan) -> Result<()> {
    let (tmp, args) = stage(plan)?;
    let result = run_escalated(&args);
    let _ = std::fs::remove_file(&tmp);
    result
}

fn run_escalated(args: &[String]) -> Result<()> {
    let (program, full_args): (String, Vec<String>) = match detect_escalator() {
        Escalator::Direct => (args[0].clone(), args[1..].to_vec()),
        Escalator::Pkexec => ("pkexec".to_string(), args.to_vec()),
        Escalator::Sudo => {
            let mut v = vec![args[0].clone()];
            v.extend_from_slice(&args[1..]);
            ("sudo".to_string(), v)
        }
    };

    let output = Command::new(&program)
        .args(&full_args)
        .output()
        .with_context(|| format!("running {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "privileged apply failed (exit {}):\n{}{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        );
    }
    Ok(())
}

/// The privileged side: executed as root via the `apply` subcommand. Performs
/// validation, backup, atomic write, and the service action. Must be robust —
/// this is the only code that touches root-owned config and the live daemon.
pub fn apply_main(
    dest: &Path,
    src: &Path,
    restart: Option<&str>,
    service_spec: Option<&str>,
) -> Result<()> {
    let new_contents = std::fs::read_to_string(src)
        .with_context(|| format!("reading staged config {}", src.display()))?;

    // Re-validate as root; never write a config we know is broken.
    let cf = ConfigFile::parse(&new_contents);
    let findings = validate(&cf);
    if validate::has_errors(&findings) {
        let msgs: Vec<String> = findings
            .iter()
            .filter(|f| f.severity == crate::config::Severity::Error)
            .map(|f| f.message.clone())
            .collect();
        bail!("refusing to write config with errors:\n  {}", msgs.join("\n  "));
    }

    // Backup existing file if present.
    if dest.exists() {
        let ts = unix_timestamp();
        let backup = dest.with_extension(format!("conf.bak-{ts}"));
        std::fs::copy(dest, &backup)
            .with_context(|| format!("backing up to {}", backup.display()))?;
    }

    // Atomic write: temp in the destination directory, then rename.
    let dir = dest.parent().unwrap_or(Path::new("/etc/apcupsd"));
    let tmp = dir.join(format!(
        ".apctui-write-{}.tmp",
        dest.file_name().and_then(|s| s.to_str()).unwrap_or("conf")
    ));
    std::fs::write(&tmp, new_contents.as_bytes())
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("renaming into place {}", dest.display()))?;

    // Service action.
    if let Some(name) = restart {
        systemctl(&["restart", &service::unit_for(name)])
            .with_context(|| format!("restarting apcupsd@{name}"))?;
    }
    if let Some(spec) = service_spec {
        let (verb, name) = spec
            .split_once(':')
            .context("--service expects verb:name")?;
        systemctl(&[verb, &service::unit_for(name)])
            .with_context(|| format!("{verb} apcupsd@{name}"))?;
    }
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .context("running systemctl")?;
    if !status.success() {
        bail!("systemctl {:?} exited {}", args, status.code().unwrap_or(-1));
    }
    Ok(())
}

/// A standalone service action (start/stop/etc.) with no config change.
pub fn run_service_action(action: ServiceAction, name: &str) -> Result<()> {
    let exe = self_exe()?;
    let args = vec![
        exe.to_string_lossy().into_owned(),
        "service".to_string(),
        "--action".to_string(),
        action.verb().to_string(),
        "--name".to_string(),
        name.to_string(),
    ];
    run_escalated(&args)
}

/// Privileged side of a bare service action.
pub fn service_main(action: &str, name: &str) -> Result<()> {
    systemctl(&[action, &service::unit_for(name)])
        .with_context(|| format!("{action} apcupsd@{name}"))
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
