// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! The Gerber and Excellon readers and the board reader are fed random and mutated input: they
//! may reject it, they may never panic, and whatever they accept the writers can handle again.

mod common;
use circuito_pcb::board::Board;
use circuito_pcb::excellon::{read_excellon, write_drills, DrillOptions};
use circuito_pcb::gerber::{read_gerber, write_all, GerberOptions};
use common::*;
use proptest::prelude::*;

fn samples() -> (Vec<String>, Vec<String>) {
    let b = rich_board();
    let g = write_all(&b, &GerberOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| f.content)
        .collect();
    let d = write_drills(&b, &DrillOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| f.content)
        .collect();
    (g, d)
}

fn mutate(s: &str, edits: &[(usize, u8, u8)]) -> String {
    let mut bytes = s.as_bytes().to_vec();
    for (pos, op, val) in edits {
        if bytes.is_empty() {
            break;
        }
        let i = pos % bytes.len();
        match op % 3 {
            0 => bytes[i] = *val,
            1 => {
                bytes.remove(i);
            }
            _ => bytes.insert(i, *val),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn gerber_reader_survives_mutation(file in 0usize..9, edits in prop::collection::vec((any::<usize>(), any::<u8>(), any::<u8>()), 1..12)) {
        let (g, _) = samples();
        let text = mutate(&g[file % g.len()], &edits);
        let _ = read_gerber(&text);
    }

    #[test]
    fn excellon_reader_survives_mutation(edits in prop::collection::vec((any::<usize>(), any::<u8>(), any::<u8>()), 1..12)) {
        let (_, d) = samples();
        let text = mutate(&d[0], &edits);
        let _ = read_excellon(&text);
    }

    #[test]
    fn readers_survive_random_text(s in "\\PC{0,400}") {
        let _ = read_gerber(&s);
        let _ = read_excellon(&s);
        let _ = Board::from_json(&s);
    }

    #[test]
    fn readers_survive_random_command_soup(parts in prop::collection::vec("[GDMXYIJT0-9%*.,;-]{0,14}", 0..40)) {
        let s = parts.join("\n");
        let _ = read_gerber(&s);
        let _ = read_excellon(&format!("M48\nMETRIC\n%\n{s}"));
    }
}

#[test]
fn what_the_writer_wrote_always_reads_back() {
    let (g, d) = samples();
    for t in &g {
        let img = read_gerber(t).unwrap();
        assert!(img.finished);
        // a second pass over the bytes is identical: the reader is deterministic
        assert_eq!(img, read_gerber(t).unwrap());
    }
    for t in &d {
        assert!(read_excellon(t).unwrap().finished);
    }
}

#[test]
fn oversized_input_is_refused_before_parsing() {
    let big = "G04 x*\n".repeat(40 * 1024 * 1024);
    assert!(big.len() > circuito_pcb::gerber::MAX_GERBER_BYTES);
    let e = read_gerber(&big).unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
}
