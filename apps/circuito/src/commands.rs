// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything the person can ask for is a command: the menus, the shortcuts, the command search
//! and the buttons all run these.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::tr;

/// Where a command is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    File,
    View,
    Simulation,
    Go,
    App,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::File,
        Category::View,
        Category::Simulation,
        Category::Go,
        Category::App,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::View => "View",
            Category::Simulation => "Simulation",
            Category::Go => "Go",
            Category::App => "Application",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Open,
    Reload,
    CloseDocument,
    Home,
    FitView,
    ZoomIn,
    ZoomOut,
    ToggleGrid,
    ShowSchematic,
    ShowNetlist,
    ToggleNavigator,
    ToggleProperties,
    ToggleBottomPanel,
    ShowProblems,
    ShowSimulation,
    Run,
    RunAll,
    Stop,
    NextProblem,
    PreviousProblem,
    NextDocument,
    PreviousDocument,
    CommandSearch,
    Preferences,
    KeyboardShortcuts,
    About,
    Quit,
}

const fn s(modifiers: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(modifiers, key))
}

const CMD: Modifiers = Modifiers::COMMAND;
const NONE: Modifiers = Modifiers::NONE;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CMD_SHIFT: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: true,
};

impl Cmd {
    pub const ALL: [Cmd; 27] = [
        Cmd::Open,
        Cmd::Reload,
        Cmd::CloseDocument,
        Cmd::Home,
        Cmd::FitView,
        Cmd::ZoomIn,
        Cmd::ZoomOut,
        Cmd::ToggleGrid,
        Cmd::ShowSchematic,
        Cmd::ShowNetlist,
        Cmd::ToggleNavigator,
        Cmd::ToggleProperties,
        Cmd::ToggleBottomPanel,
        Cmd::ShowProblems,
        Cmd::ShowSimulation,
        Cmd::Run,
        Cmd::RunAll,
        Cmd::Stop,
        Cmd::NextProblem,
        Cmd::PreviousProblem,
        Cmd::NextDocument,
        Cmd::PreviousDocument,
        Cmd::CommandSearch,
        Cmd::Preferences,
        Cmd::KeyboardShortcuts,
        Cmd::About,
        Cmd::Quit,
    ];

    /// The stable id the global menu and automation use.
    pub fn id(self) -> &'static str {
        match self {
            Cmd::Open => "file.open",
            Cmd::Reload => "file.reload",
            Cmd::CloseDocument => "file.close",
            Cmd::Home => "go.home",
            Cmd::FitView => "view.fit",
            Cmd::ZoomIn => "view.zoom_in",
            Cmd::ZoomOut => "view.zoom_out",
            Cmd::ToggleGrid => "view.grid",
            Cmd::ShowSchematic => "view.schematic",
            Cmd::ShowNetlist => "view.netlist",
            Cmd::ToggleNavigator => "view.navigator",
            Cmd::ToggleProperties => "view.properties",
            Cmd::ToggleBottomPanel => "view.bottom_panel",
            Cmd::ShowProblems => "view.problems",
            Cmd::ShowSimulation => "view.simulation",
            Cmd::Run => "sim.run",
            Cmd::RunAll => "sim.run_all",
            Cmd::Stop => "sim.stop",
            Cmd::NextProblem => "go.next_problem",
            Cmd::PreviousProblem => "go.previous_problem",
            Cmd::NextDocument => "go.next_document",
            Cmd::PreviousDocument => "go.previous_document",
            Cmd::CommandSearch => "app.search",
            Cmd::Preferences => "app.preferences",
            Cmd::KeyboardShortcuts => "app.shortcuts",
            Cmd::About => "app.about",
            Cmd::Quit => "app.quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// The English label (translated where it is drawn).
    pub fn label(self) -> &'static str {
        match self {
            Cmd::Open => "Open…",
            Cmd::Reload => "Reload from disk",
            Cmd::CloseDocument => "Close circuit",
            Cmd::Home => "Home",
            Cmd::FitView => "Fit to window",
            Cmd::ZoomIn => "Zoom in",
            Cmd::ZoomOut => "Zoom out",
            Cmd::ToggleGrid => "Show the grid",
            Cmd::ShowSchematic => "Show the schematic",
            Cmd::ShowNetlist => "Show the netlist",
            Cmd::ToggleNavigator => "Show the navigator",
            Cmd::ToggleProperties => "Show the properties",
            Cmd::ToggleBottomPanel => "Show the results panel",
            Cmd::ShowProblems => "Show the rules check",
            Cmd::ShowSimulation => "Show the simulation",
            Cmd::Run => "Run the analysis",
            Cmd::RunAll => "Run every analysis",
            Cmd::Stop => "Stop the simulation",
            Cmd::NextProblem => "Next finding",
            Cmd::PreviousProblem => "Previous finding",
            Cmd::NextDocument => "Next circuit",
            Cmd::PreviousDocument => "Previous circuit",
            Cmd::CommandSearch => "Search commands…",
            Cmd::Preferences => "Preferences",
            Cmd::KeyboardShortcuts => "Keyboard shortcuts",
            Cmd::About => "About Circuito",
            Cmd::Quit => "Quit",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Cmd::Open | Cmd::Reload | Cmd::CloseDocument => Category::File,
            Cmd::FitView
            | Cmd::ZoomIn
            | Cmd::ZoomOut
            | Cmd::ToggleGrid
            | Cmd::ShowSchematic
            | Cmd::ShowNetlist
            | Cmd::ToggleNavigator
            | Cmd::ToggleProperties
            | Cmd::ToggleBottomPanel
            | Cmd::ShowProblems
            | Cmd::ShowSimulation => Category::View,
            Cmd::Run | Cmd::RunAll | Cmd::Stop => Category::Simulation,
            Cmd::Home
            | Cmd::NextProblem
            | Cmd::PreviousProblem
            | Cmd::NextDocument
            | Cmd::PreviousDocument => Category::Go,
            Cmd::CommandSearch
            | Cmd::Preferences
            | Cmd::KeyboardShortcuts
            | Cmd::About
            | Cmd::Quit => Category::App,
        }
    }

    /// The shortcut, when it has one. Plain keys (no modifier) only act when no text field has
    /// the keyboard.
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        match self {
            Cmd::Open => s(CMD, Key::O),
            Cmd::Reload => s(CMD, Key::R),
            Cmd::CloseDocument => s(CMD, Key::W),
            Cmd::Home => s(CMD_SHIFT, Key::H),
            Cmd::FitView => s(NONE, Key::F),
            Cmd::ZoomIn => s(CMD, Key::Equals),
            Cmd::ZoomOut => s(CMD, Key::Minus),
            Cmd::ToggleGrid => s(NONE, Key::G),
            Cmd::ShowSchematic => s(CMD, Key::Num1),
            Cmd::ShowNetlist => s(CMD, Key::Num2),
            Cmd::ToggleNavigator => s(CMD, Key::B),
            Cmd::ToggleProperties => s(CMD_SHIFT, Key::B),
            Cmd::ToggleBottomPanel => s(CMD, Key::J),
            Cmd::ShowProblems => s(CMD, Key::Num3),
            Cmd::ShowSimulation => s(CMD, Key::Num4),
            Cmd::Run => s(NONE, Key::F5),
            Cmd::RunAll => s(SHIFT, Key::F5),
            Cmd::Stop => None,
            Cmd::NextProblem => s(NONE, Key::F8),
            Cmd::PreviousProblem => s(SHIFT, Key::F8),
            Cmd::NextDocument => s(CMD, Key::Tab),
            Cmd::PreviousDocument => s(CMD_SHIFT, Key::Tab),
            Cmd::CommandSearch => s(CMD, Key::K),
            Cmd::Preferences => s(CMD, Key::Comma),
            Cmd::KeyboardShortcuts => s(CMD, Key::Slash),
            Cmd::About => None,
            Cmd::Quit => s(CMD, Key::Q),
        }
    }

    /// The label in the current language.
    pub fn translated(self) -> &'static str {
        tr(self.label())
    }

    /// The icon in menus and the command search (an NDL icon name), when it has one.
    pub fn icon(self) -> Option<&'static str> {
        Some(match self {
            Cmd::Open => "open",
            Cmd::Reload => "refresh",
            Cmd::CloseDocument => "close",
            Cmd::Home => "home",
            Cmd::FitView => "fit",
            Cmd::ZoomIn => "zoom",
            Cmd::ZoomOut => "zoom-out",
            Cmd::ToggleGrid => "grid",
            Cmd::ShowSchematic => "sketch",
            Cmd::ShowNetlist => "file-text",
            Cmd::ToggleNavigator => "navigator",
            Cmd::ToggleProperties => "properties",
            Cmd::ToggleBottomPanel => "panel-bottom",
            Cmd::ShowProblems => "shield-check",
            Cmd::ShowSimulation => "waveform",
            Cmd::Run => "run",
            Cmd::RunAll => "play",
            Cmd::Stop => "stop",
            Cmd::NextProblem => "arrow-down",
            Cmd::PreviousProblem => "arrow-up",
            Cmd::NextDocument => "chevron-right",
            Cmd::PreviousDocument => "chevron-left",
            Cmd::CommandSearch => "search",
            Cmd::Preferences => "settings",
            Cmd::KeyboardShortcuts => "keyboard",
            Cmd::About => "info",
            Cmd::Quit => "log-out",
        })
    }

    /// The shortcut as the platform writes it (`Ctrl+S`, `⌘S`).
    pub fn shortcut_label(self, mac: bool) -> Option<String> {
        self.shortcut().map(|s| label_of(&s, mac))
    }

    /// Whether the shortcut has no modifier, so a text field must keep it for typing.
    pub fn is_plain_key(self) -> bool {
        self.shortcut().is_some_and(|k| k.modifiers == NONE)
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Enter => "Enter".to_owned(),
        Key::Delete => "Delete".to_owned(),
        Key::ArrowUp => "↑".to_owned(),
        Key::ArrowDown => "↓".to_owned(),
        Key::Comma => ",".to_owned(),
        Key::Slash => "/".to_owned(),
        Key::Equals => "+".to_owned(),
        Key::Minus => "-".to_owned(),
        Key::Num1 => "1".to_owned(),
        Key::Num2 => "2".to_owned(),
        Key::Num3 => "3".to_owned(),
        Key::Num4 => "4".to_owned(),
        other => other.name().to_owned(),
    }
}

/// How a shortcut is written on this platform.
pub fn label_of(s: &KeyboardShortcut, mac: bool) -> String {
    let m = s.modifiers;
    let key = key_name(s.logical_key);
    if mac {
        let mut out = String::new();
        if m.alt {
            out.push('⌥');
        }
        if m.shift {
            out.push('⇧');
        }
        if m.command {
            out.push('⌘');
        }
        out.push_str(&key);
        out
    } else {
        let mut parts: Vec<&str> = Vec::new();
        if m.command {
            parts.push("Ctrl");
        }
        if m.alt {
            parts.push("Alt");
        }
        if m.shift {
            parts.push("Shift");
        }
        let mut out = parts.join("+");
        if !out.is_empty() {
            out.push('+');
        }
        out.push_str(&key);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_found_again() {
        let mut seen = HashSet::new();
        for c in Cmd::ALL {
            assert!(seen.insert(c.id()), "{} twice", c.id());
            assert_eq!(Cmd::from_id(c.id()), Some(c));
        }
        assert_eq!(Cmd::from_id("nope"), None);
    }

    #[test]
    fn no_two_commands_share_a_shortcut() {
        let mut seen = HashSet::new();
        for c in Cmd::ALL {
            if let Some(k) = c.shortcut() {
                assert!(
                    seen.insert((k.modifiers.command, k.modifiers.shift, k.logical_key)),
                    "{}: {k:?} is taken",
                    c.id()
                );
            }
        }
    }

    #[test]
    fn every_category_is_used_and_every_command_has_one() {
        for cat in Category::ALL {
            assert!(Cmd::ALL.iter().any(|c| c.category() == cat), "{cat:?}");
        }
    }

    #[test]
    fn shortcuts_are_written_for_the_platform() {
        assert_eq!(Cmd::Open.shortcut_label(false).as_deref(), Some("Ctrl+O"));
        assert_eq!(
            Cmd::Home.shortcut_label(false).as_deref(),
            Some("Ctrl+Shift+H")
        );
        assert_eq!(Cmd::Home.shortcut_label(true).as_deref(), Some("⇧⌘H"));
        assert_eq!(Cmd::Run.shortcut_label(false).as_deref(), Some("F5"));
        assert_eq!(Cmd::About.shortcut_label(false), None);
    }

    #[test]
    fn every_icon_exists_in_ndl() {
        for c in Cmd::ALL {
            if let Some(icon) = c.icon() {
                assert!(
                    nodalia_theme::icons::get(icon).is_some(),
                    "{c:?}: no NDL icon called {icon}"
                );
            }
        }
    }

    #[test]
    fn the_command_list_is_complete() {
        // Adding a command without listing it in ALL would leave it with no menu entry.
        assert_eq!(Cmd::ALL.len(), 27);
    }
}
