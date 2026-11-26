// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito's desktop app: schematics and SPICE decks to open, check and simulate, native egui,
//! drawn in the Nodalia Design Language.
//!
//! `CIRCUITO_CONFIG_DIR` keeps the preferences somewhere else than the platform's config folder,
//! which is how two copies run side by side on one computer.

// A window app on Windows: no console opens beside it.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod app;
mod canvas;
mod chrome;
#[cfg(target_os = "linux")]
mod linux_menu;
mod menu;
mod overlays;
mod panels;
mod plotview;
mod ui;
mod widgets;

use std::path::PathBuf;
use std::process::ExitCode;

use circuito_app::settings::Settings;

fn main() -> ExitCode {
    let mut open: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("circuito-app {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: circuito-app [FILE.circuito | FILE.cir]...\n\nCIRCUITO_CONFIG_DIR=<folder> keeps the preferences in <folder>."
                );
                return ExitCode::SUCCESS;
            }
            path => open.push(path.into()),
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let settings = Settings::load();
    let ndl_controls = chrome::NDL_CONTROLS_POSSIBLE && !settings.native_window_controls;
    let mut viewport = chrome::frame(
        egui::ViewportBuilder::default()
            .with_title(circuito_app::NAME)
            .with_app_id(circuito_app::APP_ID)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([860.0, 560.0]),
        ndl_controls,
    );
    viewport = viewport.with_icon(egui::IconData {
        rgba: circuito_app::icon::rgba(256),
        width: 256,
        height: 256,
    });
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        circuito_app::NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, settings, open)) as Box<dyn eframe::App>)),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("circuito-app: {e}");
            ExitCode::FAILURE
        }
    }
}
