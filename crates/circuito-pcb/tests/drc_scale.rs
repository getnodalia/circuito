// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! DRC on a large board (20,000 tracks) and on hostile boards: it must finish quickly, find the
//! one planted fault, and refuse or survive absurd numbers without panicking.

mod common;
use std::time::Instant;

use circuito_pcb::board::*;
use circuito_pcb::drc::{run_drc, Rules, Severity, ViolationCode as C};
use circuito_pcb::layer::Layer;
use circuito_pcb::units::{MAX_COORD, MM};
use common::*;

const ROWS: i64 = 100;
const COLS: i64 = 200;
const PITCH_Y: i64 = 350_000;
const PITCH_X: i64 = 600_000;

/// 100 rows of 200 overlapping 0.15 mm tracks (20,000 in all), neighbouring rows on different
/// nets and 0.20 mm apart, so a clearance of 0.20 mm is met exactly everywhere.
fn big_board() -> Board {
    let mut b = Board::new("big", 2).unwrap();
    b.outline = Some(Outline::rectangle(0, 0, 130 * MM, 40 * MM));
    for (i, n) in ["A", "B", "C", "GND"].iter().enumerate() {
        b.nets.push(Net {
            id: NetId(i as u32 + 1),
            name: n.to_string(),
            class: DEFAULT_CLASS.to_string(),
        });
    }
    for i in 0..ROWS {
        let net = Some(NetId((i % 4) as u32 + 1));
        let y = MM + i * PITCH_Y;
        for j in 0..COLS {
            let x = MM + j * PITCH_X;
            track(&mut b, net, Layer::FCu, 150_000, p(x, y), p(x + 500_000, y));
        }
    }
    b
}

fn rules() -> Rules {
    let mut r = Rules::default();
    r.clearance = circuito_pcb::drc::ClearanceTable::uniform(200_000);
    r.min_track_width = 150_000;
    r.copper_to_edge = 300_000;
    r.check_dangling = false;
    r
}

#[test]
fn twenty_thousand_tracks_check_quickly_and_exactly() {
    let mut b = big_board();
    assert_eq!(b.tracks.len(), 20_000);
    let t0 = Instant::now();
    let clean = run_drc(&b, &rules()).unwrap();
    let dt = t0.elapsed();
    eprintln!("DRC of 20,000 tracks: {dt:?}, {:?}", clean.stats);
    assert!(
        clean
            .violations
            .iter()
            .all(|v| v.severity != Severity::Error),
        "{:?}",
        clean.violations.iter().take(3).collect::<Vec<_>>()
    );
    assert!(dt.as_secs_f64() < 10.0, "took {dt:?}");
    assert_eq!(clean.stats.copper_items, 20_000);

    // plant one fault: a track of net A whose copper is 1 nm closer than the clearance to the
    // top row (net GND): centre distance = 75 um + 75 um + 199,999 nm
    let y = MM + (ROWS - 1) * PITCH_Y + 349_999;
    track(
        &mut b,
        NA,
        Layer::FCu,
        150_000,
        p(30 * MM, y),
        p(30 * MM + 100_000, y),
    );
    let t1 = Instant::now();
    let r = run_drc(&b, &rules()).unwrap();
    eprintln!("DRC with the fault: {:?}", t1.elapsed());
    let hits: Vec<_> = r
        .violations
        .iter()
        .filter(|v| v.code == C::ClearanceTrackTrack)
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].required, Some(200_000));
    assert_eq!(hits[0].measured, Some(199_999));
}

#[test]
fn hostile_boards_are_errors_not_panics() {
    // a coordinate beyond the supported range is refused at validation
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(0, 0),
        p(MAX_COORD + 1, 0),
    );
    assert!(run_drc(&b, &Rules::default()).is_err());
    // so is an absurd width
    let mut b = base_board();
    track(&mut b, NA, Layer::FCu, i64::MAX, p(0, 0), p(MM, 0));
    assert!(run_drc(&b, &Rules::default()).is_err());
    // negative or zero width
    let mut b = base_board();
    track(&mut b, NA, Layer::FCu, 0, p(0, 0), p(MM, 0));
    assert!(run_drc(&b, &Rules::default()).is_err());
    // rules with absurd values
    let mut r = Rules::default();
    r.min_track_width = i64::MAX;
    assert!(run_drc(&base_board(), &r).is_err());
    let mut r = Rules::default();
    r.clearance = circuito_pcb::drc::ClearanceTable::uniform(i64::MIN);
    assert!(run_drc(&base_board(), &r).is_err());
    // extreme but legal coordinates and sizes at the edge of the range do not overflow
    let mut b = base_board();
    b.outline = Some(Outline::rectangle(
        -MAX_COORD, -MAX_COORD, MAX_COORD, MAX_COORD,
    ));
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(-MAX_COORD, -MAX_COORD),
        p(MAX_COORD, MAX_COORD),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(-MAX_COORD, MAX_COORD),
        p(MAX_COORD, -MAX_COORD),
    );
    let r = run_drc(&b, &Rules::default()).unwrap();
    assert!(r.has(C::ShortCircuit), "{:?}", r.violations);
}
