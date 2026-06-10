// SPDX-License-Identifier: GPL-3.0-or-later
//! Line-level diff between two config texts, for the confirm-before-save view.
//! A minimal LCS diff keeps the preview readable and dependency-free.

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLine {
    Context(String),
    Removed(String),
    Added(String),
}

pub fn diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let n = a.len();
    let m = b.len();

    // LCS length table.
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    // Backtrack into a diff script.
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine::Context(a[i].to_string()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine::Removed(a[i].to_string()));
            i += 1;
        } else {
            out.push(DiffLine::Added(b[j].to_string()));
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine::Removed(a[i].to_string()));
        i += 1;
    }
    while j < m {
        out.push(DiffLine::Added(b[j].to_string()));
        j += 1;
    }
    out
}

/// Collapse long runs of unchanged context to keep previews compact,
/// keeping `pad` lines around each change.
pub fn compact(lines: &[DiffLine], pad: usize) -> Vec<DiffLine> {
    let changed: Vec<bool> = lines
        .iter()
        .map(|l| !matches!(l, DiffLine::Context(_)))
        .collect();
    let keep: Vec<bool> = (0..lines.len())
        .map(|i| {
            let lo = i.saturating_sub(pad);
            let hi = (i + pad).min(lines.len() - 1);
            changed[lo..=hi].iter().any(|&c| c)
        })
        .collect();

    let mut out = Vec::new();
    let mut skipping = false;
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            out.push(line.clone());
            skipping = false;
        } else if !skipping {
            out.push(DiffLine::Context("…".to_string()));
            skipping = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_change() {
        let d = diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains(&DiffLine::Removed("b".into())));
        assert!(d.contains(&DiffLine::Added("B".into())));
        assert_eq!(d.iter().filter(|l| matches!(l, DiffLine::Context(_))).count(), 2);
    }

    #[test]
    fn identical_is_all_context() {
        let d = diff("x\ny\n", "x\ny\n");
        assert!(d.iter().all(|l| matches!(l, DiffLine::Context(_))));
    }

    #[test]
    fn compaction_collapses_context() {
        let old = (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let mut nv: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        nv[25] = "changed".into();
        let d = diff(&old, &nv.join("\n"));
        let c = compact(&d, 2);
        assert!(c.len() < d.len());
        assert!(c.iter().any(|l| matches!(l, DiffLine::Added(s) if s == "changed")));
    }
}
