// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writes the app icon as raw RGBA, for `packaging/icons.sh`.
//!
//! `cargo run -p circuito-app --example render_icon -- <size> <out.rgba>` writes `size * size * 4`
//! bytes of straight (not premultiplied) RGBA, the same pixels the window shows. The script turns
//! them into the PNG, ICO and ICNS files the installers carry.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable
)]

use std::process::ExitCode;

/// The largest icon asked for: 4096 px is 64 MiB of pixels, far more than an icon needs.
const MAX_SIZE: usize = 4096;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [size, out] = args.as_slice() else {
        eprintln!("usage: render_icon <size> <out.rgba>");
        return ExitCode::from(2);
    };
    let size = match size.parse::<usize>() {
        Ok(n) if (1..=MAX_SIZE).contains(&n) => n,
        _ => {
            eprintln!("render_icon: size must be a whole number from 1 to {MAX_SIZE}");
            return ExitCode::from(2);
        }
    };
    match std::fs::write(out, circuito_app::icon::rgba(size)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("render_icon: cannot write {out}: {err}");
            ExitCode::FAILURE
        }
    }
}
