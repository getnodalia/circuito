// SPDX-License-Identifier: MIT OR Apache-2.0
//! The application: its state, the commands, background work and the frame loop.
//!
//! Nothing here reads a file or solves a circuit on the window's thread. Files are read on a
//! thread of their own and answer through a channel; analyses run on the simulation worker; the
//! file dialog runs on a thread too.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use circuito_app::commands::Cmd;
use circuito_app::document::{self, Circuit, Sample};
use circuito_app::editing::Tool;
use circuito_app::i18n::{self, tr, trf, Lang};
use circuito_app::settings::{Appearance, Settings};
use circuito_app::sim::Runner;
use circuito_app::workspace::{Bottom, RunError, Tab, Which, Workspace};
use circuito_core::edit::Clip;
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::{self, Appearance as NdlAppearance};
use nodalia_theme::{Accent, Mode};

use crate::{chrome, ui};

/// What covers the document.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    None,
    Search(String),
    Preferences,
    Shortcuts(String),
    About,
    /// A circuit with changes the file does not have is about to go.
    Unsaved(Pending),
}

/// What waits for the answer about unsaved changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Close the circuit with this id.
    Close(u64),
    /// Quit the application.
    Quit,
}

pub struct Toast {
    pub text: String,
    pub until: f64,
}

/// Where a file that was read goes.
enum Target {
    /// Open it as a circuit of its own.
    New,
    /// Replace the circuit with this id, which was read from the same file.
    Reload(u64),
}

/// What a background thread says.
enum Msg {
    Picked(Option<PathBuf>),
    Loaded {
        path: PathBuf,
        target: Target,
        result: Box<Result<Circuit, String>>,
    },
    /// The name chosen for a circuit that has none (or a new one).
    SavePicked {
        id: u64,
        path: Option<PathBuf>,
    },
    Saved {
        id: u64,
        path: PathBuf,
        rev: u64,
        result: Result<(), String>,
    },
}

pub struct App {
    pub settings: Settings,
    pub ws: Workspace,
    pub overlay: Overlay,
    /// Home (start cards and recent files) covers the document.
    pub home: bool,
    pub toasts: Vec<Toast>,
    /// Files being read.
    pub loading: usize,
    /// Where the pointer is on the drawing, in grid units, while it is over it.
    pub hover: Option<(f32, f32)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    dialog_open: bool,
    /// What was copied on the drawing, and how many times it was pasted since.
    clip: Clip,
    pasted: i32,
    /// What to do once the circuit being saved is on disk.
    after_save: Option<Pending>,
    /// What is typed in the parts palette's search field.
    pub part_filter: String,
    /// The window may close: nothing is left to ask about.
    quitting: bool,
    applied: Option<(Appearance, bool, bool)>,
    decorated: Option<bool>,
    #[cfg(target_os = "linux")]
    menu: Option<crate::linux_menu::GlobalMenu>,
    ctx: egui::Context,
    icon: Option<egui::TextureHandle>,
}

pub fn mode_for(dark: bool, high_contrast: bool) -> Mode {
    match (dark, high_contrast) {
        (false, false) => Mode::Light,
        (true, false) => Mode::Dark,
        (false, true) => Mode::LightHc,
        (true, true) => Mode::DarkHc,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, open: Vec<PathBuf>) -> App {
        i18n::set(Lang::from_setting(&settings.language));
        let ctx = cc.egui_ctx.clone();
        eng::install_fonts(&ctx);
        let (tx, rx) = channel();
        let wake = ctx.clone();
        let runner = Runner::spawn(move || wake.request_repaint()).ok();
        let mut app = App {
            ws: Workspace::new(runner),
            settings,
            overlay: Overlay::None,
            home: false,
            toasts: Vec::new(),
            loading: 0,
            hover: None,
            tx,
            rx,
            dialog_open: false,
            clip: Clip::default(),
            pasted: 0,
            after_save: None,
            part_filter: String::new(),
            quitting: false,
            applied: None,
            decorated: None,
            #[cfg(target_os = "linux")]
            menu: crate::linux_menu::install(
                &ctx,
                std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()),
            ),
            ctx,
            icon: None,
        };
        let ctx = app.ctx.clone();
        app.apply_appearance(&ctx);
        app.start(open);
        app
    }

    /// The first circuits: the files named on the command line, the sample on the very first
    /// launch, else Home.
    fn start(&mut self, open: Vec<PathBuf>) {
        let given: Vec<PathBuf> = open.into_iter().filter(|p| p.is_file()).collect();
        if !given.is_empty() {
            for path in given {
                self.load(path, Target::New);
            }
        } else if !self.settings.sample_shown {
            self.settings.sample_shown = true;
            self.save_settings();
            self.open_sample(Sample::RcLowPass);
        } else {
            self.home = true;
        }
    }

    pub fn ndl_controls(&self) -> bool {
        chrome::NDL_CONTROLS_POSSIBLE && !self.settings.native_window_controls
    }

    pub fn dark(&self, ctx: &egui::Context) -> bool {
        match self.settings.appearance {
            Appearance::System => ctx.theme() == egui::Theme::Dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// The colours to draw with, following the system theme when asked to.
    pub fn look(&self, ctx: &egui::Context) -> (Look, Mode) {
        let mode = mode_for(self.dark(ctx), self.settings.high_contrast);
        (Look::new(mode.palette_with(Accent::Circuito)), mode)
    }

    /// The icon as a texture, made once.
    pub fn icon(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.icon
            .get_or_insert_with(|| {
                // A missing icon draws as one transparent pixel rather than failing.
                let (w, h, rgba) = circuito_app::icon::rgba(circuito_app::icon::SMALL)
                    .unwrap_or_else(|| (1, 1, vec![0; 4]));
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                ctx.load_texture("circuito-icon", image, egui::TextureOptions::LINEAR)
            })
            .id()
    }

    fn apply_appearance(&mut self, ctx: &egui::Context) {
        let key = (
            self.settings.appearance,
            self.settings.high_contrast,
            self.settings.reduced_motion,
        );
        if self.applied == Some(key) {
            return;
        }
        self.applied = Some(key);
        let appearance = |dark: bool| NdlAppearance {
            mode: mode_for(dark, self.settings.high_contrast),
            accent: Accent::Circuito,
            reduced_motion: self.settings.reduced_motion,
            ..NdlAppearance::default()
        };
        match self.settings.appearance {
            Appearance::System => for_egui::apply_system(ctx, &appearance(false)),
            Appearance::Light => for_egui::apply(ctx, &appearance(false)),
            Appearance::Dark => for_egui::apply(ctx, &appearance(true)),
        }
        eng::apply_text_styles(ctx);
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push(Toast {
            text: text.into(),
            until: now + 4.0,
        });
        self.ctx.request_repaint();
    }

    /// Save the preferences on a thread of their own.
    pub fn save_settings(&self) {
        self.settings.save_in_background(|result| {
            if let Err(e) = result {
                tracing::warn!("could not save the preferences: {e}");
            }
        });
    }

    // --- Files --------------------------------------------------------------------------------

    /// Read `path` on a thread of its own.
    fn load(&mut self, path: PathBuf, target: Target) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let read = path.clone();
        let spawned = std::thread::Builder::new()
            .name("open-file".into())
            .spawn(move || {
                let result = Box::new(Circuit::open(&read));
                let _ = tx.send(Msg::Loaded {
                    path: read,
                    target,
                    result,
                });
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.loading += 1,
            Err(_) => {
                let name = document::file_name(&path);
                self.toast(trf(
                    "Could not open {}: {}",
                    &[&name, tr("no thread to read it")],
                ));
            }
        }
    }

    fn pick_open(&mut self) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let title = tr("Open a circuit");
        let filter = tr("Circuits and SPICE decks");
        let extensions: Vec<&str> = document::SCHEMATIC_EXTENSIONS
            .iter()
            .chain(document::SCRIPT_EXTENSIONS.iter())
            .chain(document::SPICE_EXTENSIONS.iter())
            .copied()
            .collect();
        let spawned = std::thread::Builder::new()
            .name("open-dialog".into())
            .spawn(move || {
                let picked = pollster::block_on(
                    rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter(filter, &extensions)
                        .pick_file(),
                )
                .map(|f| f.path().to_path_buf());
                let _ = tx.send(Msg::Picked(picked));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.dialog_open = false;
            self.toast(tr("Could not open the file dialog."));
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        self.load(path, Target::New);
    }

    pub fn open_sample(&mut self, sample: Sample) {
        self.ws.open(Circuit::sample(sample));
        self.opened();
    }

    /// What follows a circuit coming up as the shown one.
    fn opened(&mut self) {
        self.home = false;
        let has_drawing = self
            .ws
            .active()
            .is_some_and(|d| d.circuit.schematic.is_some());
        self.ws.tab = if has_drawing {
            Tab::Schematic
        } else {
            Tab::Netlist
        };
        // A circuit the rules check refuses opens on what it found, not on a plot it cannot make.
        let failed = self.ws.active().is_some_and(|d| d.counts().0 > 0);
        self.ws.bottom = if failed {
            Bottom::Rules
        } else {
            Bottom::Simulation
        };
        if self.settings.auto_run {
            // A circuit that cannot run says why in the simulation panel; nothing to announce.
            let _ = self.ws.run(Which::All);
        }
    }

    // --- Saving -------------------------------------------------------------------------------

    /// Save circuit `id`; ask for a name first when it has none (or `as_new`).
    pub fn save_doc(&mut self, id: u64, as_new: bool) {
        let now = self.ctx.input(|i| i.time);
        let Some(doc) = self.ws.docs_mut().iter_mut().find(|d| d.id == id) else {
            return;
        };
        if !doc.editable() {
            self.after_save = None;
            self.toast(tr("Only drawn circuits can be saved here."));
            return;
        }
        // What is in the code pane counts, even if the person has not paused yet.
        doc.script_settle(now, true);
        if !doc.script.is_clean() {
            self.after_save = None;
            self.toast(tr("Fix the problems in the code first."));
            return;
        }
        doc.refresh();
        let known = doc
            .circuit
            .path
            .clone()
            .filter(|p| document::is_writable_extension(p));
        match known {
            Some(path) if !as_new => self.write(id, path),
            other => self.pick_save(id, other),
        }
    }

    fn pick_save(&mut self, id: u64, current: Option<PathBuf>) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let title = tr("Save the circuit");
        let name = self
            .ws
            .docs()
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.circuit.name())
            .unwrap_or_default();
        let name = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .map_or_else(|| tr("Circuit").to_owned(), str::to_owned);
        let (drawn, text) = (tr("Circuit"), tr("Circuit as code"));
        let spawned = std::thread::Builder::new()
            .name("save-dialog".into())
            .spawn(move || {
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name(format!("{name}.circuito"))
                    .add_filter(drawn, &document::SCHEMATIC_EXTENSIONS)
                    .add_filter(text, &document::SCRIPT_EXTENSIONS);
                if let Some(dir) = current.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(dir);
                }
                let picked = pollster::block_on(dialog.save_file())
                    .map(|f| document::with_extension_if_missing(f.path()));
                let _ = tx.send(Msg::SavePicked { id, path: picked });
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.dialog_open = false;
            self.after_save = None;
            self.toast(tr("Could not open the file dialog."));
        }
    }

    /// Write circuit `id` to `path` on a thread of its own, through a temporary sibling.
    fn write(&mut self, id: u64, path: PathBuf) {
        let Some(doc) = self.ws.docs().iter().find(|d| d.id == id) else {
            self.after_save = None;
            return;
        };
        let rev = doc.rev;
        let text = match doc.circuit.schematic.as_ref() {
            Some(s) => document::serialise(s, &path),
            None => Err(tr("There is no drawing to save.").to_owned()),
        };
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                self.after_save = None;
                self.toast(trf(
                    "Could not save {}: {}",
                    &[&document::file_name(&path), &e],
                ));
                return;
            }
        };
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let target = path.clone();
        let spawned = std::thread::Builder::new()
            .name("save-file".into())
            .spawn(move || {
                let result = document::write_atomically(&target, &text);
                let _ = tx.send(Msg::Saved {
                    id,
                    path: target,
                    rev,
                    result,
                });
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.after_save = None;
            self.toast(tr("Could not start saving."));
        }
    }

    fn saved(&mut self, id: u64, path: PathBuf, rev: u64, result: Result<(), String>) {
        let name = document::file_name(&path);
        if let Err(e) = result {
            self.after_save = None;
            self.toast(trf("Could not save {}: {}", &[&name, &e]));
            return;
        }
        if let Some(doc) = self.ws.docs_mut().iter_mut().find(|d| d.id == id) {
            doc.saved_rev = rev;
            doc.circuit.path = Some(path.clone());
            doc.circuit.sample = None;
        }
        self.settings.remember(&path);
        self.save_settings();
        self.toast(trf("Saved {}", &[&name]));
        match self.after_save.take() {
            Some(Pending::Close(close)) if close == id => self.close_id(id),
            _ => {}
        }
    }

    /// Close circuit `id` without asking.
    fn close_id(&mut self, id: u64) {
        if let Some(i) = self.ws.docs().iter().position(|d| d.id == id) {
            self.ws.close(i);
            if self.ws.docs().is_empty() {
                self.home = true;
            }
        }
    }

    /// Close circuit `id`, asking first when it has changes the file does not.
    pub fn request_close(&mut self, id: u64) {
        let dirty = self
            .ws
            .docs()
            .iter()
            .find(|d| d.id == id)
            .is_some_and(|d| d.dirty());
        if dirty {
            self.overlay = Overlay::Unsaved(Pending::Close(id));
        } else {
            self.close_id(id);
        }
    }

    /// Quit, asking first when something would be lost.
    fn request_quit(&mut self) {
        if self.ws.docs().iter().any(|d| d.dirty()) {
            self.overlay = Overlay::Unsaved(Pending::Quit);
        } else {
            self.quit_now();
        }
    }

    fn quit_now(&mut self) {
        self.quitting = true;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The answer "save" to the question about closing one circuit.
    pub fn save_then_close(&mut self, id: u64) {
        self.after_save = Some(Pending::Close(id));
        self.save_doc(id, false);
    }

    /// The answer "discard" to the question about unsaved changes.
    pub fn discard(&mut self, pending: Pending) {
        match pending {
            Pending::Close(id) => self.close_id(id),
            Pending::Quit => self.quit_now(),
        }
    }

    fn poll(&mut self) {
        if self.ws.poll() {
            self.ctx.request_repaint();
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Picked(path) => {
                    self.dialog_open = false;
                    if let Some(path) = path {
                        self.load(path, Target::New);
                    }
                }
                Msg::Loaded {
                    path,
                    target,
                    result,
                } => {
                    self.loading = self.loading.saturating_sub(1);
                    self.loaded(path, target, *result);
                }
                Msg::SavePicked { id, path } => {
                    self.dialog_open = false;
                    match path {
                        Some(path) => self.write(id, path),
                        None => self.after_save = None,
                    }
                }
                Msg::Saved {
                    id,
                    path,
                    rev,
                    result,
                } => self.saved(id, path, rev, result),
            }
        }
    }

    fn loaded(&mut self, path: PathBuf, target: Target, result: Result<Circuit, String>) {
        let name = document::file_name(&path);
        let circuit = match result {
            Ok(c) => c,
            Err(message) => {
                self.settings.recent.retain(|p| p != &path);
                self.save_settings();
                self.toast(trf("Could not open {}: {}", &[&name, &message]));
                return;
            }
        };
        match target {
            Target::New => {
                self.ws.open(circuit);
                self.settings.remember(&path);
                self.save_settings();
                self.opened();
            }
            Target::Reload(id) => {
                let Some(index) = self.ws.docs().iter().position(|d| d.id == id) else {
                    return;
                };
                self.ws.replace(index, circuit);
                self.toast(trf("Reloaded {}", &[&name]));
                if self.settings.auto_run && self.ws.active_index() == Some(index) {
                    let _ = self.ws.run(Which::All);
                }
            }
        }
    }

    // --- Commands -----------------------------------------------------------------------------

    fn run_analyses(&mut self, which: Which) {
        self.settings.show_results = true;
        self.ws.bottom = Bottom::Simulation;
        match self.ws.run(which) {
            Ok(_) => {}
            Err(RunError::NoCircuit) => self.toast(tr("There is no circuit to simulate.")),
            Err(RunError::NoAnalyses) => {
                self.toast(tr(
                    "This circuit asks for no analysis (.op, .dc, .ac or .tran).",
                ));
            }
            Err(RunError::Blocked(reason)) => {
                self.toast(trf("This circuit cannot be simulated: {}", &[&reason]));
            }
            Err(RunError::WorkerLost) => {
                self.toast(tr("The simulation engine stopped. Restart Circuito."));
            }
        }
    }

    /// The shown circuit, when it is one that can be drawn on.
    fn editable(&mut self) -> Option<&mut circuito_app::workspace::Open> {
        if self.ws.tab() != Tab::Schematic {
            return None;
        }
        self.ws.active_mut().filter(|d| d.editable())
    }

    /// Carry out a change from the properties panel; a refusal is said in a toast.
    pub fn apply(&mut self, action: circuito_app::editing::Action) {
        let result = self.ws.active_mut().map(|d| d.apply(action));
        if let Some(Err(message)) = result {
            self.toast(message);
        }
    }

    /// Choose what a click on the drawing does.
    pub fn use_tool(&mut self, tool: Tool) {
        if self.ws.active().is_some_and(|d| d.editable()) {
            self.ws.tab = Tab::Schematic;
            self.home = false;
            self.ws.tool = tool;
        }
    }

    fn zoom(&mut self, factor: f32) {
        if let Some(doc) = self.ws.active_mut() {
            doc.view = doc.view.zoomed_at(factor, (1.0, 1.0), (2.0, 2.0));
        }
    }

    fn show_results(&mut self, bottom: Bottom) {
        self.settings.show_results = true;
        self.ws.bottom = bottom;
        self.save_settings();
    }

    fn toggle(&mut self, pick: fn(&mut Settings) -> &mut bool) {
        let flag = pick(&mut self.settings);
        *flag = !*flag;
        self.save_settings();
    }

    pub fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Open => self.pick_open(),
            Cmd::Reload => {
                let target = self
                    .ws
                    .active()
                    .and_then(|d| d.circuit.path.clone().map(|p| (d.id, p)));
                match target {
                    Some((id, path)) => self.load(path, Target::Reload(id)),
                    None => self.toast(tr(
                        "This circuit is not a file, so there is nothing to reload.",
                    )),
                }
            }
            Cmd::CloseDocument => {
                if let Some(id) = self.ws.active().map(|d| d.id) {
                    self.request_close(id);
                }
            }
            Cmd::New => {
                self.ws.open(Circuit::blank());
                self.opened();
                self.ws.tab = Tab::Schematic;
            }
            Cmd::Save | Cmd::SaveAs => {
                if let Some(id) = self.ws.active().map(|d| d.id) {
                    self.save_doc(id, cmd == Cmd::SaveAs);
                }
            }
            Cmd::Undo => {
                if let Some(doc) = self.editable() {
                    if doc.undo().is_none() {
                        self.toast(tr("Nothing to undo."));
                    }
                }
            }
            Cmd::Redo => {
                if let Some(doc) = self.editable() {
                    if doc.redo().is_none() {
                        self.toast(tr("Nothing to redo."));
                    }
                }
            }
            Cmd::Cut | Cmd::Copy => {
                if let Some(doc) = self.editable() {
                    let clip = doc.copy_selection();
                    if clip.is_empty() {
                        return;
                    }
                    if cmd == Cmd::Cut {
                        doc.delete_selection();
                    }
                    self.clip = clip;
                    // A cut pastes back in place the first time; a copy lands beside the original.
                    self.pasted = if cmd == Cmd::Cut { -1 } else { 0 };
                }
            }
            Cmd::Paste => {
                self.pasted += 1;
                let offset = PASTE_STEP * self.pasted;
                let clip = self.clip.clone();
                if let Some(doc) = self.editable() {
                    doc.paste(&clip, offset);
                }
            }
            Cmd::Duplicate => {
                if let Some(doc) = self.editable() {
                    doc.duplicate();
                }
            }
            Cmd::Delete => {
                if let Some(doc) = self.editable() {
                    doc.delete_selection();
                }
            }
            Cmd::SelectAll => {
                if let Some(doc) = self.editable() {
                    doc.select_all();
                }
            }
            Cmd::RotateRight | Cmd::RotateLeft => {
                // What is in the hand turns before anything on the drawing does.
                if matches!(self.ws.tool, Tool::Part(_) | Tool::Power(_)) {
                    self.ws.ghost = self.ws.ghost.turned(cmd == Cmd::RotateLeft);
                } else if let Some(doc) = self.editable() {
                    doc.rotate_selection(cmd == Cmd::RotateLeft);
                }
            }
            Cmd::MirrorHorizontal | Cmd::MirrorVertical => {
                if matches!(self.ws.tool, Tool::Part(_)) {
                    self.ws.ghost.mirror = !self.ws.ghost.mirror;
                } else if let Some(doc) = self.editable() {
                    doc.flip_selection(cmd == Cmd::MirrorHorizontal);
                }
            }
            Cmd::ToolSelect => self.use_tool(Tool::Select),
            Cmd::ToolWire => self.use_tool(Tool::Wire),
            Cmd::ToolLabel => self.use_tool(Tool::Label),
            Cmd::ToolPower => self.use_tool(Tool::Power("GND".to_owned())),
            Cmd::ToolNoConnect => self.use_tool(Tool::NoConnect),
            Cmd::ToggleScript => self.toggle(|s| &mut s.show_script),
            Cmd::Home => self.home = true,
            Cmd::FitView => {
                if let Some(doc) = self.ws.active_mut() {
                    doc.fit_pending = true;
                }
            }
            Cmd::ZoomIn => self.zoom(1.25),
            Cmd::ZoomOut => self.zoom(0.8),
            Cmd::ToggleGrid => self.toggle(|s| &mut s.show_grid),
            Cmd::ShowSchematic => {
                if self
                    .ws
                    .active()
                    .is_some_and(|d| d.circuit.schematic.is_some())
                {
                    self.ws.tab = Tab::Schematic;
                    self.home = false;
                }
            }
            Cmd::ShowNetlist => {
                self.ws.tab = Tab::Netlist;
                self.home = false;
            }
            Cmd::ToggleNavigator => self.toggle(|s| &mut s.show_navigator),
            Cmd::ToggleProperties => self.toggle(|s| &mut s.show_properties),
            Cmd::ToggleBottomPanel => self.toggle(|s| &mut s.show_results),
            Cmd::ShowProblems => self.show_results(Bottom::Rules),
            Cmd::ShowSimulation => self.show_results(Bottom::Simulation),
            Cmd::Run => self.run_analyses(Which::Chosen),
            Cmd::RunAll => self.run_analyses(Which::All),
            Cmd::Stop => self.ws.stop(),
            Cmd::NextProblem | Cmd::PreviousProblem => {
                self.home = false;
                self.show_results(Bottom::Rules);
                self.ws.step_finding(cmd == Cmd::NextProblem);
            }
            Cmd::NextDocument => {
                self.ws.step(true);
                self.home = false;
            }
            Cmd::PreviousDocument => {
                self.ws.step(false);
                self.home = false;
            }
            Cmd::CommandSearch => self.overlay = Overlay::Search(String::new()),
            Cmd::Preferences => self.overlay = Overlay::Preferences,
            Cmd::KeyboardShortcuts => self.overlay = Overlay::Shortcuts(String::new()),
            Cmd::About => self.overlay = Overlay::About,
            Cmd::Quit => self.request_quit(),
        }
    }

    /// Runs the commands whose shortcut was pressed this frame.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.overlay != Overlay::None {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        // Most specific first: egui matches a shortcut even when Shift or Alt are also held.
        let mut table: Vec<(Cmd, egui::KeyboardShortcut)> = Cmd::ALL
            .iter()
            .filter(|c| !(typing && (c.is_plain_key() || c.is_text_edit_key())))
            .filter_map(|c| c.shortcut().map(|s| (*c, s)))
            .collect();
        table.sort_by_key(|(_, s)| {
            std::cmp::Reverse(u8::from(s.modifiers.shift) + u8::from(s.modifiers.alt))
        });
        let mut commands = Vec::new();
        ctx.input_mut(|i| {
            for (cmd, s) in &table {
                if i.consume_shortcut(s) {
                    commands.push(*cmd);
                }
            }
        });
        // Backspace deletes like Delete, except in a text field.
        if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
        {
            commands.push(Cmd::Delete);
        }
        for cmd in commands {
            self.run(cmd);
        }
    }

    /// Keeping the circuit, its text form and its results in step with each other.
    fn sync(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let holding = ctx.input(|i| i.pointer.any_down());
        let auto_run = self.settings.auto_run;
        let Some(doc) = self.ws.active_mut() else {
            return;
        };
        let mut again = false;
        if doc.script.typed_at.is_some() {
            if doc.script_settle(now, false) {
                again = true;
            } else if doc.script.typed_at.is_some() {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
        }
        if doc.refresh() {
            again = true;
        }
        let run = auto_run && doc.run_pending && !holding && doc.script.typed_at.is_none();
        if run {
            doc.run_pending = false;
        }
        if run {
            // A circuit that cannot run says why in the simulation panel; nothing to announce.
            let _ = self.ws.run(Which::All);
            again = true;
        }
        if again {
            ctx.request_repaint();
        }
    }

    /// Circuits dropped on the window open.
    fn dropped(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in files {
            self.load(path, Target::New);
        }
    }
}

/// How far each further paste lands from the one before, in grid units.
const PASTE_STEP: i32 = 4;

/// A folder's name for a path, for the status bar and Home.
pub fn folder_of(path: &Path) -> String {
    path.parent()
        .map_or_else(String::new, |p| p.display().to_string())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_appearance(&ctx);
        self.poll();

        let decorated = !self.ndl_controls();
        if self.decorated != Some(decorated) {
            if self.decorated.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(decorated));
            }
            self.decorated = Some(decorated);
        }

        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.ws.docs().iter().any(|d| d.dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.overlay = Overlay::Unsaved(Pending::Quit);
        }

        self.dropped(&ctx);
        self.shortcuts(&ctx);
        self.sync(&ctx);
        ui::show(ui, self);

        #[cfg(target_os = "linux")]
        if let Some(menu) = &self.menu {
            for id in menu.take() {
                if let Some(cmd) = Cmd::from_id(id) {
                    self.run(cmd);
                }
            }
        }

        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| t.until > now);
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // A running analysis moves its progress bar.
        if self.ws.busy() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.window_fill.to_normalized_gamma_f32()
    }
}
