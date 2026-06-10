// SPDX-License-Identifier: GPL-3.0-or-later
//! Round-trip parser for apcupsd.conf.
//!
//! apcupsd.conf is line-oriented: `DIRECTIVE value`, `#` comments, blanks.
//! We model the file as an ordered list of lines, each tagged as a comment/
//! blank (opaque, preserved verbatim) or a directive (key + value + original
//! formatting). This lets us edit one directive's value without disturbing
//! comments, ordering, or whitespace elsewhere — essential when writing back
//! a root-owned file the user has hand-annotated.
//!
//! Invariant (tested): for any input, `parse(s).serialize() == s`.

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// Comment or blank line, preserved exactly (no trailing newline).
    Verbatim(String),
    Directive {
        key: String,
        value: String,
        /// Whitespace between key and value (preserved on untouched lines).
        sep: String,
        /// Leading indentation, if any.
        indent: String,
        /// Trailing content after the value (comments, spaces).
        trailing: String,
        /// Set when the value has been edited; forces canonical reserialize.
        dirty: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ConfigFile {
    pub lines: Vec<Line>,
    /// True if the original text ended with a newline.
    pub trailing_newline: bool,
}

fn split_indent(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_whitespace()).unwrap_or(s.len());
    s.split_at(end)
}

impl ConfigFile {
    pub fn parse(input: &str) -> Self {
        let trailing_newline = input.ends_with('\n');

        // split('\n') on "a\n" yields ["a", ""]; the synthetic trailing empty
        // segment is dropped so a final newline doesn't fabricate a blank line.
        let mut raw_lines: Vec<&str> = input.split('\n').collect();
        if trailing_newline {
            raw_lines.pop();
        }

        let parsed = raw_lines
            .into_iter()
            .map(|raw| {
                let (indent, rest) = split_indent(raw);
                let trimmed = rest.trim_start();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                    return Line::Verbatim(raw.to_string());
                }
                // DIRECTIVE value[ trailing]
                let key_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let key = rest[..key_end].to_string();
                let after = &rest[key_end..];
                let sep_end = after
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(after.len());
                let sep = after[..sep_end].to_string();
                let value = after[sep_end..].to_string();
                // A bare directive ("DEVICE") has no value and no separator;
                // preserve that exactly. Only directives with a value carry a
                // separator. (sep is whatever whitespace actually followed.)
                Line::Directive {
                    key,
                    value,
                    sep,
                    indent: indent.to_string(),
                    trailing: String::new(),
                    dirty: false,
                }
            })
            .collect();

        ConfigFile { lines: parsed, trailing_newline }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            match line {
                Line::Verbatim(s) => out.push_str(s),
                Line::Directive { key, value, sep, indent, trailing, .. } => {
                    out.push_str(indent);
                    out.push_str(key);
                    out.push_str(sep);
                    out.push_str(value);
                    out.push_str(trailing);
                }
            }
        }
        if self.trailing_newline {
            out.push('\n');
        }
        out
    }

    /// First value for `key` (case-insensitive), if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|l| match l {
            Line::Directive { key: k, value, .. } if k.eq_ignore_ascii_case(key) => {
                Some(value.as_str())
            }
            _ => None,
        })
    }

    /// All directive (key, value) pairs in file order.
    pub fn directives(&self) -> Vec<(String, String)> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Directive { key, value, .. } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    /// Set `key` to `value`. Updates the first existing occurrence (preserving
    /// its indentation), else appends a new directive line. Returns true if an
    /// existing line was modified.
    pub fn set(&mut self, key: &str, new_value: &str) -> bool {
        for line in &mut self.lines {
            if let Line::Directive { key: k, value, dirty, sep, .. } = line {
                if k.eq_ignore_ascii_case(key) {
                    if value != new_value {
                        *value = new_value.to_string();
                        if sep.is_empty() {
                            *sep = " ".to_string();
                        }
                        *dirty = true;
                    }
                    return true;
                }
            }
        }
        // Not found: append. Ensure file had a trailing newline so the new
        // line sits on its own row.
        self.trailing_newline = true;
        self.lines.push(Line::Directive {
            key: key.to_string(),
            value: new_value.to_string(),
            sep: " ".to_string(),
            indent: String::new(),
            trailing: String::new(),
            dirty: true,
        });
        false
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "## apcupsd.conf v1\n#\nUPSNAME rack-main\nUPSCABLE usb\nUPSTYPE usb\nDEVICE\nBATTERYLEVEL 10\nMINUTES 5\n\n# network\nNETSERVER on\nNISPORT 3551\n";

    #[test]
    fn roundtrips_byte_identical() {
        let cases = [
            REAL,
            "",
            "\n",
            "UPSNAME foo",
            "UPSNAME foo\n",
            "  INDENTED value\n",
            "KEY    multiple   spaces   in   value\n",
            "# just a comment\n\n\n",
            "; semicolon comment\nDEVICE /dev/ttyS0\n",
        ];
        for c in cases {
            let parsed = ConfigFile::parse(c);
            assert_eq!(parsed.serialize(), c, "roundtrip failed for {c:?}");
        }
    }

    #[test]
    fn reads_values() {
        let cf = ConfigFile::parse(REAL);
        assert_eq!(cf.get("UPSNAME"), Some("rack-main"));
        assert_eq!(cf.get("upsname"), Some("rack-main")); // case-insensitive
        assert_eq!(cf.get("MINUTES"), Some("5"));
        assert_eq!(cf.get("DEVICE"), Some("")); // present but empty
        assert_eq!(cf.get("NOPE"), None);
    }

    #[test]
    fn edits_only_target_line() {
        let mut cf = ConfigFile::parse(REAL);
        assert!(cf.set("BATTERYLEVEL", "15"));
        let out = cf.serialize();
        assert!(out.contains("BATTERYLEVEL 15"));
        // everything else intact
        assert!(out.contains("## apcupsd.conf v1\n"));
        assert!(out.contains("UPSNAME rack-main\n"));
        assert!(out.contains("# network\n"));
        // only one line changed
        let diff: Vec<_> = REAL.lines().zip(out.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0], ("BATTERYLEVEL 10", "BATTERYLEVEL 15"));
    }

    #[test]
    fn set_unchanged_value_is_clean() {
        let mut cf = ConfigFile::parse(REAL);
        cf.set("MINUTES", "5"); // same value
        assert_eq!(cf.serialize(), REAL); // untouched
    }

    #[test]
    fn appends_missing_directive() {
        let mut cf = ConfigFile::parse("UPSNAME foo\n");
        assert!(!cf.set("TIMEOUT", "0")); // false = appended
        assert_eq!(cf.serialize(), "UPSNAME foo\nTIMEOUT 0\n");
    }

    #[test]
    fn preserves_indented_value_edit() {
        let mut cf = ConfigFile::parse("  BATTERYLEVEL 10\n");
        cf.set("BATTERYLEVEL", "20");
        assert_eq!(cf.serialize(), "  BATTERYLEVEL 20\n");
    }

}
