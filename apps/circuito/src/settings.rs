// SPDX-License-Identifier: MIT OR Apache-2.0
//! The person's preferences, kept in `settings.json` in the config folder
//! (`CIRCUITO_CONFIG_DIR` overrides it, which is how two copies run side by side). A missing or
//! damaged file gives the defaults; saving writes a temporary file and renames it, off the
//! window's thread.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Largest settings file read, in bytes.
const MAX_BYTES: u64 = 1 << 20;
/// How many recent files are kept.
pub const MAX_RECENT: usize = 8;

/// Light, dark, or whatever the system uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    /// Linux: use the desktop's own window buttons instead of NDL's.
    pub native_window_controls: bool,
    /// `auto`, `es` or `en`.
    pub language: String,
    /// Run a circuit's analyses as soon as it opens.
    pub auto_run: bool,
    pub show_grid: bool,
    pub show_navigator: bool,
    pub show_properties: bool,
    pub show_results: bool,
    /// The circuit's text form beside the drawing.
    pub show_script: bool,
    /// The sample circuit was opened on the first launch.
    pub sample_shown: bool,
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: Appearance::System,
            high_contrast: false,
            reduced_motion: false,
            native_window_controls: false,
            language: "auto".into(),
            auto_run: true,
            show_grid: true,
            show_navigator: true,
            show_properties: true,
            show_results: true,
            show_script: true,
            sample_shown: false,
            recent: Vec::new(),
        }
    }
}

impl Settings {
    /// The settings file, when a config folder can be found.
    pub fn file() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("CIRCUITO_CONFIG_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir).join("settings.json"));
        }
        directories::ProjectDirs::from("dev", "danielmiguelt", "Circuito")
            .map(|d| d.config_dir().join("settings.json"))
    }

    /// Read the settings, or the defaults when there is no usable file.
    pub fn load() -> Settings {
        Self::file()
            .and_then(|p| Self::read(&p).ok())
            .unwrap_or_default()
    }

    /// Read a settings file: bounded, and an error when it is not valid.
    pub fn read(path: &Path) -> Result<Settings, String> {
        let mut text = String::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text))
            .map_err(|e| e.to_string())?;
        let mut s: Settings = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        s.recent.truncate(MAX_RECENT);
        Ok(s)
    }

    /// Write the settings to `path` through a temporary sibling.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                e.to_string()
            })
    }

    /// Save in a thread of its own; the result comes back through `done`.
    pub fn save_in_background(&self, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let Some(path) = Self::file() else {
            done(Err("no config folder".into()));
            return;
        };
        let copy = self.clone();
        let spawned = std::thread::Builder::new()
            .name("save-settings".into())
            .spawn(move || done(copy.write(&path)));
        // A thread that cannot start loses the save; the next change tries again.
        let _ = spawned;
    }

    /// Put `path` first in the recent files.
    pub fn remember(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        let mut s = Settings {
            appearance: Appearance::Dark,
            language: "es".into(),
            auto_run: false,
            ..Settings::default()
        };
        s.remember(Path::new("/tmp/a.circuito"));
        s.write(&path).unwrap();
        assert_eq!(Settings::read(&path).unwrap(), s);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_missing_field_takes_its_default_and_junk_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"appearance":"dark"}"#).unwrap();
        let s = Settings::read(&path).unwrap();
        assert_eq!(s.appearance, Appearance::Dark);
        assert!(s.auto_run && s.show_grid);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Settings::read(&path).is_err());
        std::fs::write(&path, r#"{"appearance":"purple"}"#).unwrap();
        assert!(Settings::read(&path).is_err());
        assert!(Settings::read(&dir.path().join("none.json")).is_err());
    }

    #[test]
    fn a_huge_file_is_not_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        let mut text = String::from(r#"{"language":""#);
        text.push_str(&"x".repeat(3 << 20));
        text.push_str(r#""}"#);
        std::fs::write(&path, text).unwrap();
        assert!(Settings::read(&path).is_err(), "cut short, so not valid");
    }

    #[test]
    fn recent_files_are_unique_newest_first_and_capped() {
        let mut s = Settings::default();
        for i in 0..20 {
            s.remember(Path::new(&format!("/f/{i}.circuito")));
        }
        s.remember(Path::new("/f/15.circuito"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], Path::new("/f/15.circuito"));
        assert_eq!(
            s.recent
                .iter()
                .filter(|p| *p == Path::new("/f/15.circuito"))
                .count(),
            1
        );
    }
}
