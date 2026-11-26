// SPDX-License-Identifier: MIT OR Apache-2.0
//! Printing analysis results as aligned tables, CSV or JSON.

use circuito_sim::{AnalysisResult, Signal, Values};
use serde_json::{json, Value};
use std::io::{self, Write};

fn num(v: f64) -> String {
    // `-0.0` prints as "-0.0e0", which only confuses a reader; zero has no sign here.
    let v = if v == 0.0 { 0.0 } else { v };
    format!("{v:.9e}")
}

/// One printable column: a header and its values.
struct Column {
    header: String,
    cells: Vec<String>,
}

fn real_cells(vals: &[f64]) -> Vec<String> {
    vals.iter().map(|v| num(*v)).collect()
}

fn columns(r: &AnalysisResult) -> Vec<Column> {
    let t = &r.table;
    let mut cols = vec![Column {
        header: t.axis_name.clone(),
        cells: real_cells(&t.axis),
    }];
    for s in &t.signals {
        match &s.values {
            Values::Real(v) => cols.push(Column {
                header: s.name.clone(),
                cells: real_cells(v),
            }),
            Values::Complex(_) => {
                let n = s.len();
                let get = |f: fn(&Signal, usize) -> Option<f64>| {
                    (0..n)
                        .map(|i| f(s, i).map_or_else(|| "nan".to_string(), num))
                        .collect::<Vec<_>>()
                };
                cols.push(Column {
                    header: format!("mag({})", s.name),
                    cells: get(Signal::magnitude),
                });
                cols.push(Column {
                    header: format!("db({})", s.name),
                    cells: get(Signal::db),
                });
                cols.push(Column {
                    header: format!("phase_deg({})", s.name),
                    cells: get(Signal::phase_deg),
                });
            }
        }
    }
    cols
}

fn heading(r: &AnalysisResult) -> String {
    let i = &r.info;
    let mut s = format!("{}  [{}; dc: {}", i.analysis, i.engine, i.dc_method);
    if let Some(m) = &i.integration {
        s.push_str(&format!("; {m}"));
    }
    s.push(']');
    s
}

/// Print `r` as an aligned table. `.op` is printed one signal per line.
pub fn table(w: &mut dyn Write, r: &AnalysisResult) -> io::Result<()> {
    writeln!(w, "* {}", heading(r))?;
    for warn in &r.info.warnings {
        writeln!(w, "* warning: {warn}")?;
    }
    let cols = columns(r);
    let rows = cols.first().map_or(0, |c| c.cells.len());
    if r.table.axis_name == "op" {
        let width = r
            .table
            .signals
            .iter()
            .map(|s| s.name.len())
            .max()
            .unwrap_or(0);
        for c in cols.iter().skip(1) {
            let unit = r
                .table
                .signals
                .iter()
                .find(|s| s.name == c.header)
                .map_or("", |s| match s.unit {
                    circuito_sim::Unit::Volt => " V",
                    circuito_sim::Unit::Ampere => " A",
                });
            writeln!(
                w,
                "{:<width$}  {}{unit}",
                c.header,
                c.cells.first().map_or("", String::as_str)
            )?;
        }
        return Ok(());
    }
    let widths: Vec<usize> = cols
        .iter()
        .map(|c| {
            c.cells
                .iter()
                .map(String::len)
                .chain([c.header.len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |w: &mut dyn Write, f: &dyn Fn(usize) -> String| -> io::Result<()> {
        let parts: Vec<String> = cols
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(k, (_, wd))| format!("{:<wd$}", f(k)))
            .collect();
        writeln!(w, "{}", parts.join("  ").trim_end())
    };
    line(w, &|k| {
        cols.get(k).map_or(String::new(), |c| c.header.clone())
    })?;
    for row in 0..rows {
        line(w, &|k| {
            cols.get(k)
                .and_then(|c| c.cells.get(row))
                .cloned()
                .unwrap_or_default()
        })?;
    }
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Print `r` as CSV, with a leading `# ...` comment naming the analysis.
pub fn csv(w: &mut dyn Write, r: &AnalysisResult) -> io::Result<()> {
    writeln!(w, "# {}", heading(r))?;
    for warn in &r.info.warnings {
        writeln!(w, "# warning: {warn}")?;
    }
    let cols = columns(r);
    let rows = cols.first().map_or(0, |c| c.cells.len());
    let head: Vec<String> = cols.iter().map(|c| csv_field(&c.header)).collect();
    writeln!(w, "{}", head.join(","))?;
    for row in 0..rows {
        let cells: Vec<&str> = cols
            .iter()
            .map(|c| c.cells.get(row).map_or("", String::as_str))
            .collect();
        writeln!(w, "{}", cells.join(","))?;
    }
    Ok(())
}

fn f(v: f64) -> Value {
    // Results are finite by construction (the simulator errors on a non-finite number); the
    // fallback keeps the output valid JSON regardless.
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

fn signal_json(s: &Signal) -> Value {
    let unit = match s.unit {
        circuito_sim::Unit::Volt => "V",
        circuito_sim::Unit::Ampere => "A",
    };
    match &s.values {
        Values::Real(v) => json!({
            "name": s.name, "unit": unit,
            "values": v.iter().map(|x| f(*x)).collect::<Vec<_>>(),
        }),
        Values::Complex(_) => {
            let n = s.len();
            let col = |g: fn(&Signal, usize) -> Option<f64>| -> Vec<Value> {
                (0..n).map(|i| g(s, i).map_or(Value::Null, f)).collect()
            };
            json!({
                "name": s.name, "unit": unit,
                "magnitude": col(Signal::magnitude),
                "db": col(Signal::db),
                "phase_deg": col(Signal::phase_deg),
            })
        }
    }
}

/// The JSON value for one analysis result.
#[must_use]
pub fn result_json(r: &AnalysisResult) -> Value {
    let i = &r.info;
    json!({
        "analysis": i.analysis,
        "axis": { "name": r.table.axis_name, "values": r.table.axis.iter().map(|x| f(*x)).collect::<Vec<_>>() },
        "signals": r.table.signals.iter().map(signal_json).collect::<Vec<_>>(),
        "info": {
            "engine": i.engine,
            "dc_method": i.dc_method,
            "integration": i.integration,
            "reltol": f(i.reltol), "abstol": f(i.abstol), "vntol": f(i.vntol),
            "gmin": f(i.gmin), "temp": f(i.temp),
            "newton_iterations": i.newton_iterations,
            "factorizations": i.factorizations,
            "accepted_steps": i.accepted_steps,
            "rejected_steps": i.rejected_steps,
            "warnings": i.warnings,
        },
    })
}
