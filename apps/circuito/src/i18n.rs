// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text in Spanish and English.
//!
//! The source is written in English, through [`tr`]; Spanish comes from [`SPANISH`]. A test reads
//! every `tr("…")` in `src/` and every command label and fails when one has no Spanish entry, so a
//! new string cannot ship untranslated. Text with a value in it uses `{}` placeholders filled by
//! [`trf`].

use std::sync::atomic::{AtomicU8, Ordering};

/// The languages the interface speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// The system language when it is Spanish, English otherwise.
    pub fn system() -> Lang {
        if system_tag().is_some_and(|tag| is_spanish(&tag)) {
            Lang::Es
        } else {
            Lang::En
        }
    }

    /// The language a settings value names: `es`, `en`, or anything else for the system's.
    pub fn from_setting(value: &str) -> Lang {
        match value {
            "es" => Lang::Es,
            "en" => Lang::En,
            _ => Lang::system(),
        }
    }
}

fn system_tag() -> Option<String> {
    let platform = || sys_locale::get_locale().filter(|tag| !tag.is_empty() && tag.len() <= 64);
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return platform();
    }
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .or_else(platform)
}

/// Whether a locale tag (`es`, `es-ES`, `es_MX.UTF-8`, `es-419`) names Spanish.
fn is_spanish(tag: &str) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    tag.strip_prefix("es")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_', '.', '@']))
}

static CURRENT: AtomicU8 = AtomicU8::new(1);

/// Choose the language of every later [`tr`].
pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Es => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

/// The language in use.
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::Es,
        _ => Lang::En,
    }
}

/// The text in the current language.
pub fn tr(english: &'static str) -> &'static str {
    if current() == Lang::En {
        return english;
    }
    SPANISH
        .binary_search_by(|(en, _)| en.cmp(&english))
        .ok()
        .and_then(|i| SPANISH.get(i))
        .map_or(english, |(_, es)| es)
}

/// [`tr`] with each `{}` replaced, in order, by `args`.
pub fn trf(english: &'static str, args: &[&str]) -> String {
    fill(tr(english), args)
}

/// `template` with each `{}` replaced, in order, by `args`.
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for arg in args {
        match rest.split_once("{}") {
            Some((before, after)) => {
                out.push_str(before);
                out.push_str(arg);
                rest = after;
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// English → Spanish, sorted by the English text (the lookup is a binary search).
pub use crate::spanish::SPANISH;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        for pair in SPANISH.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{:?} is out of order", pair[1].0);
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        for (en, es) in SPANISH {
            assert_eq!(
                en.matches("{}").count(),
                es.matches("{}").count(),
                "{en}: placeholders differ"
            );
        }
    }

    #[test]
    fn no_spanish_entry_is_empty_or_identical_by_accident() {
        for (en, es) in SPANISH {
            assert!(!es.trim().is_empty(), "{en}: empty translation");
        }
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(fill("Line {}, column {}", &["3", "9"]), "Line 3, column 9");
        assert_eq!(fill("no holes", &["x"]), "no holes");
        assert_eq!(fill("{} and {}", &["a"]), "a and {}");
        assert_eq!(fill("{}", &["{}", "z"]), "{}");
    }

    #[test]
    fn spanish_tags() {
        for tag in ["es", "es-ES", "es_MX.UTF-8", "es-419", "ES"] {
            assert!(is_spanish(tag), "{tag}");
        }
        for tag in ["en", "en_US", "eu", "est", ""] {
            assert!(!is_spanish(tag), "{tag}");
        }
    }

    #[test]
    fn every_command_and_category_has_spanish() {
        let has = |english: &str| SPANISH.binary_search_by(|(en, _)| en.cmp(&english)).is_ok();
        let mut missing: Vec<String> = crate::commands::Cmd::ALL
            .iter()
            .filter(|c| !has(c.label()))
            .map(|c| format!("command {}: {}", c.id(), c.label()))
            .collect();
        for cat in crate::commands::Category::ALL {
            if !has(cat.label()) {
                missing.push(format!("category: {}", cat.label()));
            }
        }
        assert!(
            missing.is_empty(),
            "without Spanish:\n{}",
            missing.join("\n")
        );
    }

    /// Every ERC rule and simulation message the app words itself has Spanish too.
    #[test]
    fn every_rule_the_app_words_has_spanish() {
        let has = |english: &str| SPANISH.binary_search_by(|(en, _)| en.cmp(&english)).is_ok();
        let missing: Vec<&str> = crate::findings::ALL
            .iter()
            .map(|r| crate::findings::english(*r))
            .filter(|t| !has(t))
            .collect();
        assert!(missing.is_empty(), "rules without Spanish: {missing:?}");
    }

    #[test]
    fn every_interface_string_has_spanish() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read file");
                for marker in ["tr(", "trf("] {
                    for (at, _) in text.match_indices(marker) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = text[at + marker.len()..].trim_start();
                        let Some(piece) = rest.strip_prefix('"') else {
                            continue;
                        };
                        let Some(end) = piece.find('"') else {
                            continue;
                        };
                        let english = &piece[..end];
                        if SPANISH
                            .binary_search_by(|(en, _)| en.cmp(&english))
                            .is_err()
                        {
                            missing.push(format!("{}: {english}", path.display()));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "strings without Spanish:\n{}",
            missing.join("\n")
        );
    }

    /// The Spanish table has no entry nothing uses: a renamed string leaves no stale row.
    #[test]
    fn every_spanish_row_is_used_somewhere() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut all = String::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|n| n != "spanish.rs" && n != "i18n.rs")
                {
                    all.push_str(&std::fs::read_to_string(&path).expect("read file"));
                }
            }
        }
        let unused: Vec<&str> = SPANISH
            .iter()
            .map(|(en, _)| *en)
            .filter(|en| !all.contains(&format!("\"{en}\"")))
            .collect();
        assert!(unused.is_empty(), "unused Spanish rows: {unused:?}");
    }
}
