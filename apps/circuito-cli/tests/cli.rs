// SPDX-License-Identifier: MIT OR Apache-2.0
//! The real `circuito` binary, run on the shipped examples and on hostile input.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("circuito-cli-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn run_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_circuito"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The child may exit before reading everything; a broken pipe here is fine.
    let _ = child.stdin.take().unwrap().write_all(stdin);
    child.wait_with_output().unwrap()
}

fn run(args: &[&str]) -> Output {
    run_stdin(args, b"")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn errtext(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

/// Parse the CSV sections that `sim --csv` prints: (comment line, header, rows).
fn csv_sections(text: &str) -> Vec<(String, Vec<String>, Vec<Vec<f64>>)> {
    let mut sections = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        assert!(line.starts_with('#'), "expected a comment, got {line}");
        let comment = line.to_string();
        while lines.peek().is_some_and(|l| l.starts_with("# warning")) {
            lines.next();
        }
        let header: Vec<String> = lines.next().unwrap().split(',').map(String::from).collect();
        let mut rows = Vec::new();
        while let Some(l) = lines.peek() {
            if l.is_empty() || l.starts_with('#') {
                break;
            }
            let row: Vec<f64> = lines
                .next()
                .unwrap()
                .split(',')
                .map(|c| c.parse().unwrap())
                .collect();
            assert_eq!(row.len(), header.len());
            rows.push(row);
        }
        sections.push((comment, header, rows));
    }
    sections
}

fn col(header: &[String], name: &str) -> usize {
    header.iter().position(|h| h == name).unwrap()
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-300)
}

// ---- sim: values against closed forms ---------------------------------------------------------

#[test]
fn schematic_divider_op_matches_the_closed_form() {
    let o = run(&["sim", &example("divider.circuito"), "--csv"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let s = csv_sections(&out(&o));
    assert_eq!(s.len(), 1);
    let (_, h, rows) = &s[0];
    // 10 V over 1k + 2.2k: V(OUT) = 10 * 2.2/3.2 = 6.875 V, I = 10/3.2k = 3.125 mA.
    assert!(rel(rows[0][col(h, "V(OUT)")], 6.875) < 1e-9);
    assert!(rel(rows[0][col(h, "I(R1)")], 3.125e-3) < 1e-9);
    assert!(rel(rows[0][col(h, "I(V1)")], -3.125e-3) < 1e-9);
}

#[test]
fn rc_lowpass_ac_follows_the_transfer_function_at_every_row() {
    let o = run(&["sim", &example("rc-lowpass.cir"), "--csv", "-a", "1"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let s = csv_sections(&out(&o));
    let (comment, h, rows) = &s[0];
    assert!(comment.contains(".ac"), "{comment}");
    let (f, m, p) = (
        col(h, "frequency"),
        col(h, "mag(V(out))"),
        col(h, "phase_deg(V(out))"),
    );
    assert_eq!(
        rows.len(),
        41,
        "10 points per decade over 4 decades, both ends"
    );
    for r in rows {
        let w_rc = 2.0 * std::f64::consts::PI * r[f] * 1e3 * 100e-9;
        let mag = 1.0 / (1.0 + w_rc * w_rc).sqrt();
        let phase = -w_rc.atan().to_degrees();
        assert!(rel(r[m], mag) < 1e-6, "f={} mag {} vs {}", r[f], r[m], mag);
        assert!((r[p] - phase).abs() < 1e-4, "f={} phase", r[f]);
    }
}

#[test]
fn rc_step_response_in_json_ends_at_the_analytic_value() {
    let o = run(&["sim", &example("rc-lowpass.cir"), "--json", "-a", "2"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let doc: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    let r = &doc["results"][0];
    assert!(r["analysis"].as_str().unwrap().starts_with(".tran"));
    let t: Vec<f64> = r["axis"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let sig = r["signals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "V(out)")
        .unwrap();
    let v: Vec<f64> = sig["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    assert_eq!(t.len(), v.len());
    assert!((t[t.len() - 1] - 1e-3).abs() < 1e-12, "ends at tstop");
    // The pulse rises in 1 ns; tau = RC = 100 us. Default tolerances: see the simulator README; the
    // error stays under 2e-3 of the 1 V step.
    for (t, v) in t.iter().zip(&v) {
        let exact = 1.0 - (-(t - 0.5e-9).max(0.0) / 1e-4).exp();
        assert!((v - exact).abs() < 2e-3, "t={t} v={v} exact={exact}");
    }
    assert_eq!(r["info"]["dc_method"], "newton");
}

#[test]
fn series_rlc_resonance_peaks_at_f0_with_the_right_q() {
    let o = run(&["sim", &example("series-rlc.cir"), "--csv", "-a", "1"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let (_, h, rows) = &csv_sections(&out(&o))[0];
    let (f, m) = (col(h, "frequency"), col(h, "mag(V(b))"));
    let (l, c, r) = (10e-3, 100e-9, 10.0);
    let mut best = (0.0, 0.0);
    for row in rows {
        let w = 2.0 * std::f64::consts::PI * row[f];
        // |Vb/Vin| = 1 / |1 - w^2 L C + j w R C|
        let re = 1.0 - w * w * l * c;
        let im = w * r * c;
        assert!(rel(row[m], 1.0 / re.hypot(im)) < 1e-6, "f={}", row[f]);
        if row[m] > best.1 {
            best = (row[f], row[m]);
        }
    }
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (l * c).sqrt());
    let q = (l / c).sqrt() / r;
    assert!(rel(best.0, f0) < 0.06, "grid peak {} vs f0 {f0}", best.0);
    // The grid has 40 points per decade, so the sampled peak is within a few percent of Q
    // only when a point lands near f0; require it to be at least half of Q and never above it.
    assert!(
        best.1 > q / 2.0 && best.1 < q * 1.001,
        "peak {} vs Q {q}",
        best.1
    );
}

#[test]
fn opamp_inverting_amplifier_matches_the_finite_gain_solution() {
    let o = run(&["sim", &example("opamp-amplifier.cir"), "--csv", "-a", "1"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let (_, h, rows) = &csv_sections(&out(&o))[0];
    // Solve the circuit exactly: Vo = k*vi with k = (-A + Rout/R2)/(1 + Rout/R2), and KCL at the
    // inverting node (Vin - vi)/R1 = (vi - Vo)/R2.
    let (a, rout, r1, r2, vin) = (1e5, 75.0, 1e3, 1e4, 0.1);
    let k = (-a + rout / r2) / (1.0 + rout / r2);
    let vi = vin / (1.0 + r1 * (1.0 - k) / r2);
    let vo = k * vi;
    assert!(rel(rows[0][col(h, "V(out)")], vo) < 1e-6);
    assert!(rel(rows[0][col(h, "V(inv)")], vi) < 1e-4);
    assert!((vo + 1.0).abs() < 1e-3, "gain is -R2/R1 to 0.1%");
}

#[test]
fn opamp_dc_sweep_is_linear_inside_the_rails_and_saturates_outside() {
    let o = run(&["sim", &example("opamp-amplifier.cir"), "--csv", "-a", "2"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let (_, h, rows) = &csv_sections(&out(&o))[0];
    let (s, vo) = (col(h, "V1"), col(h, "V(out)"));
    assert_eq!(rows.len(), 13);
    for r in rows {
        if r[s].abs() <= 1.0 {
            assert!(
                (r[vo] + 10.0 * r[s]).abs() < 0.01 * r[s].abs() + 1e-3,
                "{r:?}"
            );
        }
        assert!(
            r[vo].abs() <= 12.0 + 1e-6,
            "output stays within the +-12 V rails: {r:?}"
        );
    }
    let first = rows.first().unwrap();
    let last = rows.last().unwrap();
    assert!(
        first[vo] > 11.0 && last[vo] < -11.0,
        "saturates at both ends"
    );
}

#[test]
fn diode_rectifier_peaks_one_diode_drop_below_the_source_and_holds_the_charge() {
    let o = run(&["sim", &example("diode-rectifier.cir"), "--csv"]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let (_, h, rows) = &csv_sections(&out(&o))[0];
    let (t, v) = (col(h, "time"), col(h, "V(out)"));
    let peak = rows.iter().map(|r| r[v]).fold(f64::MIN, f64::max);
    // A 5 V peak minus a silicon diode drop of 0.6 to 0.8 V at these currents.
    assert!(peak > 4.1 && peak < 4.5, "peak {peak}");
    // After the first cycle the 10 uF capacitor never discharges below the ripple floor:
    // 1k load, tau = 10 ms, so droop over one 1 ms period is under 10 % of the peak.
    let floor = rows
        .iter()
        .filter(|r| r[t] > 1.5e-3)
        .map(|r| r[v])
        .fold(f64::MAX, f64::min);
    assert!(floor > 0.8 * peak, "floor {floor} peak {peak}");
}

// ---- erc and netlist --------------------------------------------------------------------------

#[test]
fn erc_passes_a_clean_schematic_and_reports_json() {
    let o = run(&["erc", &example("divider.circuito")]);
    assert_eq!(code(&o), 0, "{}", out(&o));
    assert!(out(&o).contains("0 error(s)"), "{}", out(&o));
    let j = run(&["erc", &example("divider.circuito"), "--json"]);
    assert_eq!(code(&j), 0);
    let doc: serde_json::Value = serde_json::from_str(&out(&j)).unwrap();
    assert!(doc["violations"].as_array().unwrap().is_empty());
}

#[test]
fn erc_fails_with_exit_1_and_names_the_unconnected_pins() {
    let mut b = circuito_core::schematic::builder::Builder::new("loose");
    b.place("R", "R1", "1k", 0, 0);
    let path = scratch("loose.circuito");
    std::fs::write(&path, b.build().to_json().unwrap()).unwrap();
    let o = run(&["erc", path.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    let text = out(&o);
    assert!(text.contains("E001") && text.contains("R1"), "{text}");
}

#[test]
fn netlist_round_trips_through_the_spice_reader_and_simulates_the_same() {
    let o = run(&["netlist", &example("divider.circuito")]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    let text = out(&o);
    assert!(
        text.contains("R1 IN OUT 1k") && text.contains("R2 OUT 0 2.2k"),
        "{text}"
    );
    let o2 = run_stdin(&["sim", "-", "--csv"], text.as_bytes());
    assert_eq!(code(&o2), 0, "{}", errtext(&o2));
    let (_, h, rows) = &csv_sections(&out(&o2))[0];
    assert!(rel(rows[0][col(h, "V(OUT)")], 6.875) < 1e-9);
}

#[test]
fn output_file_is_written_atomically_and_leaves_no_temporary() {
    let path = scratch("divider.cir");
    let _ = std::fs::remove_file(&path);
    let o = run(&[
        "netlist",
        &example("divider.circuito"),
        "-o",
        path.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", errtext(&o));
    assert!(out(&o).is_empty());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("R1 IN OUT 1k"));
    assert!(!scratch("divider.cir.tmp-circuito").exists());
}

// ---- failures say what is wrong ---------------------------------------------------------------

#[test]
fn bad_command_lines_exit_2() {
    for args in [
        &["frobnicate"][..],
        &["sim"],
        &["sim", "a", "--format", "xml"],
    ] {
        let o = run(args);
        assert_eq!(code(&o), 2, "{args:?}");
        assert!(errtext(&o).starts_with("circuito: "), "{args:?}");
    }
    assert_eq!(code(&run(&[])), 0, "no arguments prints the help");
    assert!(out(&run(&["--help"])).contains("USAGE"));
    assert!(out(&run(&["--version"])).starts_with("circuito 0."));
}

#[test]
fn missing_file_and_unreadable_input_exit_1_with_the_path() {
    let o = run(&["sim", "/no/such/file.cir"]);
    assert_eq!(code(&o), 1);
    assert!(errtext(&o).contains("/no/such/file.cir"), "{}", errtext(&o));
    let o = run_stdin(&["sim", "-"], &[0xff, 0xfe, 0x00, 0x80]);
    assert_eq!(code(&o), 1);
    assert!(errtext(&o).contains("UTF-8"), "{}", errtext(&o));
}

#[test]
fn syntax_errors_carry_the_line_number() {
    let o = run_stdin(&["sim", "-"], b"title\nR1 a 0 1k\nQ1 a b\n.op\n");
    assert_eq!(code(&o), 1);
    assert!(errtext(&o).contains("line 3"), "{}", errtext(&o));
}

#[test]
fn a_file_without_analysis_cards_says_so() {
    let o = run_stdin(&["sim", "-"], b"t\nV1 a 0 1\nR1 a 0 1k\n");
    assert_eq!(code(&o), 1);
    assert!(errtext(&o).contains("no analysis card"), "{}", errtext(&o));
}

#[test]
fn analysis_index_out_of_range_is_reported() {
    let o = run(&["sim", &example("divider.circuito"), "-a", "3"]);
    assert_eq!(code(&o), 1);
    assert!(errtext(&o).contains("1 analysis card"), "{}", errtext(&o));
}

#[test]
fn a_voltage_source_loop_names_the_elements() {
    let o = run_stdin(&["sim", "-"], b"loop\nV1 a 0 1\nV2 a 0 2\nR1 a 0 1k\n.op\n");
    assert_eq!(code(&o), 1);
    let e = errtext(&o);
    assert!(
        e.contains("V1") && e.contains("V2") && e.contains("loop"),
        "{e}"
    );
}

#[test]
fn one_failing_analysis_does_not_hide_the_others() {
    // The .ac has no source with an AC value, so it fails; the .op still runs and is printed.
    let o = run_stdin(
        &["sim", "-"],
        b"t\nV1 a 0 5\nR1 a 0 1k\n.ac dec 5 1 100\n.op\n",
    );
    assert_eq!(code(&o), 1, "partial failure is still a failure");
    assert!(errtext(&o).contains(".ac"), "{}", errtext(&o));
    assert!(out(&o).contains("V(a)"), "{}", out(&o));
}

#[test]
fn an_absurd_transient_is_refused_not_attempted() {
    let o = run_stdin(
        &["sim", "-"],
        b"t\nV1 a 0 SIN(0 1 1e9)\nR1 a 0 1k\n.tran 1p 1e6\n",
    );
    assert_eq!(code(&o), 1);
    assert!(!errtext(&o).is_empty());
}

#[test]
fn hostile_inputs_never_crash() {
    let samples: Vec<Vec<u8>> = vec![
        vec![],
        b"{".to_vec(),
        b"{\"format_version\": 1}".to_vec(),
        b"{\"format_version\": 999, \"sheets\": []}".to_vec(),
        b"[1,2,3]".to_vec(),
        b".tran\n.ac\n.dc\n".to_vec(),
        b"t\nR1 a 0 1e999\n.op\n".to_vec(),
        b"t\nC1 a 0 -1\nV1 a 0 1\n.op\n".to_vec(),
        (0..=255u8).collect(),
        "t\n".repeat(1000).into_bytes(),
    ];
    for s in &samples {
        for cmd in ["sim", "erc", "netlist"] {
            let o = run_stdin(&[cmd, "-"], s);
            let c = code(&o);
            assert!(
                c == 0 || c == 1,
                "{cmd} on {:?} exited {c}",
                String::from_utf8_lossy(s)
            );
        }
    }
}
