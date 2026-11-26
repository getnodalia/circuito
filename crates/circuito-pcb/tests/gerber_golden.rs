// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! Golden files: the exact bytes of every Gerber and Excellon file of a fixed board.
//!
//! The files in `tests/golden/` were reviewed by hand against the specifications when they were
//! created. A change in output makes this test fail and show the first differing line; after a
//! deliberate change, regenerate them with `CIRCUITO_BLESS=1 cargo test -p circuito-pcb --test
//! gerber_golden` and review the diff.

mod common;
use std::path::PathBuf;

use circuito_pcb::board::*;
use circuito_pcb::excellon::{write_drills, DrillOptions};
use circuito_pcb::geom::Arc;
use circuito_pcb::gerber::{write_all, GerberOptions};
use circuito_pcb::layer::Layer;
use circuito_pcb::units::MM;
use common::*;

/// The fixed board: `rich_board` plus silkscreen text, a reference designator and an outline cut-out.
fn golden_board() -> Board {
    let mut b = rich_board();
    b.name = "golden".into();
    b.texts.push(TextItem {
        id: fresh(),
        layer: Layer::FSilk,
        text: "CIRCUITO 1".into(),
        position: p(10 * MM, 46 * MM),
        height: MM,
        thickness: 150_000,
        rotation: 0,
        mirrored: false,
        justify: Justify::Left,
    });
    let c = p(25 * MM, 35 * MM);
    let a1 = Arc::new(p(27 * MM, 35 * MM), p(23 * MM, 35 * MM), c, true).unwrap();
    let a2 = Arc::new(p(23 * MM, 35 * MM), p(27 * MM, 35 * MM), c, true).unwrap();
    b.outline
        .as_mut()
        .unwrap()
        .cutouts
        .push(vec![OutlineSeg::Arc(a1), OutlineSeg::Arc(a2)]);
    b
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn check(name: &str, content: &str) {
    let path = golden_dir().join(name);
    if std::env::var_os("CIRCUITO_BLESS").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, content).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden file {}: {e}; run with CIRCUITO_BLESS=1",
            path.display()
        )
    });
    if want != content {
        let (n, (w, g)) = want
            .lines()
            .zip(content.lines())
            .enumerate()
            .find(|(_, (w, g))| w != g)
            .map(|(i, p)| (i + 1, p))
            .unwrap_or((
                want.lines().count().min(content.lines().count()) + 1,
                ("<end>", "<end>"),
            ));
        panic!("{name} differs from the golden file at line {n}:\n  golden: {w}\n  output: {g}");
    }
}

#[test]
fn gerber_files_match_the_golden_files() {
    let set = write_all(&golden_board(), &GerberOptions::default()).unwrap();
    assert_eq!(set.files.len(), 9);
    for f in &set.files {
        check(&f.file_name, &f.content);
    }
}

#[test]
fn excellon_files_match_the_golden_files() {
    let set = write_drills(&golden_board(), &DrillOptions::default()).unwrap();
    assert!(!set.files.is_empty());
    for f in &set.files {
        check(&f.file_name, &f.content);
    }
}
