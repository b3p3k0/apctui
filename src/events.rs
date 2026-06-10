// SPDX-License-Identifier: GPL-3.0-or-later
//! Read apcupsd event logs (`/var/log/apcupsd*.events`).
//! Milestone 2 will tail these live; for now we re-read on view entry.

use std::path::PathBuf;

pub fn event_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir("/var/log")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("apcupsd") && n.ends_with(".events"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

/// Last `max` event lines across all logs, tagged with their source file.
pub fn load_tail(max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for path in event_files() {
        let tag = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines().rev().take(max) {
                out.push(format!("[{tag}] {line}"));
            }
        }
    }
    if out.is_empty() {
        out.push("no apcupsd event logs found under /var/log".to_string());
    }
    out.truncate(max);
    out
}
