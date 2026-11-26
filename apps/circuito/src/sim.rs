// SPDX-License-Identifier: MIT OR Apache-2.0
//! Analyses on a worker thread of their own, so the window never waits for the solver.
//!
//! A [`Runner`] owns one worker. The window submits analyses, polls the answers once per frame
//! and can stop what is queued or running. The worker catches a panic from the engine and reports
//! it as a failure, and if the worker is gone every pending analysis is answered with an error
//! instead of hanging.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use circuito_core::netlist::{Analysis, Netlist};
use circuito_sim::{simulate, AnalysisResult, Control, Options, SimError};

/// One analysis to run.
struct Request {
    run: u64,
    circuit: u64,
    index: usize,
    netlist: Arc<Netlist>,
    analysis: Analysis,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
}

/// How an analysis ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Finished(Box<AnalysisResult>),
    /// The engine's message, in its own words.
    Failed(String),
    Cancelled,
}

/// What the worker says.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// The analysis left the queue and is being solved.
    Started {
        run: u64,
        circuit: u64,
        index: usize,
    },
    Done {
        run: u64,
        circuit: u64,
        index: usize,
        outcome: Outcome,
        elapsed: Duration,
    },
}

struct Pending {
    run: u64,
    circuit: u64,
    index: usize,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
}

pub struct Runner {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    pending: Vec<Pending>,
    next: u64,
    lost: bool,
}

fn store(progress: &AtomicU32, fraction: f32) {
    progress.store(fraction.to_bits(), Ordering::Relaxed);
}

impl Runner {
    /// Start the worker. `wake` is called when something changed, to repaint the window.
    ///
    /// # Errors
    /// The thread could not start.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Result<Runner, String> {
        let (tx, jobs) = channel::<Request>();
        let (replies, rx) = channel::<Reply>();
        std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || work(jobs, replies, wake))
            .map_err(|e| e.to_string())?;
        Ok(Runner {
            tx,
            rx,
            pending: Vec::new(),
            next: 1,
            lost: false,
        })
    }

    /// Queue an analysis. Returns its run number.
    ///
    /// # Errors
    /// The worker is gone.
    pub fn submit(
        &mut self,
        circuit: u64,
        index: usize,
        netlist: Arc<Netlist>,
        analysis: Analysis,
    ) -> Result<u64, String> {
        if self.lost {
            return Err("the simulation worker stopped".into());
        }
        let run = self.next;
        self.next += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU32::new(0));
        let request = Request {
            run,
            circuit,
            index,
            netlist,
            analysis,
            cancel: cancel.clone(),
            progress: progress.clone(),
        };
        if self.tx.send(request).is_err() {
            self.lost = true;
            return Err("the simulation worker stopped".into());
        }
        self.pending.push(Pending {
            run,
            circuit,
            index,
            cancel,
            progress,
        });
        Ok(run)
    }

    /// Stop everything queued or running. Each ends as [`Outcome::Cancelled`].
    pub fn stop(&self) {
        for p in &self.pending {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The fraction done (0 to 1) of a run that has not answered yet.
    pub fn progress(&self, run: u64) -> Option<f32> {
        self.pending
            .iter()
            .find(|p| p.run == run)
            .map(|p| f32::from_bits(p.progress.load(Ordering::Relaxed)).clamp(0.0, 1.0))
    }

    /// Whether anything is queued or running.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Whether the worker is gone.
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// What the worker has said since the last call.
    pub fn poll(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(reply) => {
                    if let Reply::Done { run, .. } = &reply {
                        self.pending.retain(|p| p.run != *run);
                    }
                    out.push(reply);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.lost = true;
                    for p in self.pending.drain(..) {
                        out.push(Reply::Done {
                            run: p.run,
                            circuit: p.circuit,
                            index: p.index,
                            outcome: Outcome::Failed("the simulation worker stopped".into()),
                            elapsed: Duration::ZERO,
                        });
                    }
                    break;
                }
            }
        }
        out
    }
}

fn work(jobs: Receiver<Request>, replies: Sender<Reply>, wake: impl Fn() + Send + Sync + 'static) {
    let wake = Arc::new(wake);
    for request in jobs {
        let (run, circuit, index) = (request.run, request.circuit, request.index);
        if request.cancel.load(Ordering::Relaxed) {
            let done = Reply::Done {
                run,
                circuit,
                index,
                outcome: Outcome::Cancelled,
                elapsed: Duration::ZERO,
            };
            if replies.send(done).is_err() {
                return;
            }
            wake();
            continue;
        }
        if replies
            .send(Reply::Started {
                run,
                circuit,
                index,
            })
            .is_err()
        {
            return;
        }
        wake();
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| execute(&request, wake.clone())))
            .unwrap_or_else(|_| Outcome::Failed("internal error in the simulator".into()));
        store(&request.progress, 1.0);
        let done = Reply::Done {
            run,
            circuit,
            index,
            outcome,
            elapsed: started.elapsed(),
        };
        if replies.send(done).is_err() {
            return;
        }
        wake();
    }
}

fn execute(request: &Request, wake: Arc<dyn Fn() + Send + Sync>) -> Outcome {
    let options = match Options::from_netlist(&request.netlist) {
        Ok(o) => o,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let progress = request.progress.clone();
    let report: Arc<dyn Fn(f64) + Send + Sync> = Arc::new(move |fraction| {
        let next = fraction as f32;
        let last = f32::from_bits(progress.load(Ordering::Relaxed));
        // Wake the window for every percent, not for every step of the solver.
        if next - last >= 0.01 || next >= 1.0 {
            store(&progress, next);
            wake();
        }
    });
    let control = Control::new()
        .with_cancel(request.cancel.clone())
        .with_progress(report);
    match simulate(&request.netlist, &request.analysis, &options, &control) {
        Ok(result) => Outcome::Finished(Box::new(result)),
        Err(SimError::Cancelled) => Outcome::Cancelled,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Circuit, Sample};
    use circuito_sim::Kind;

    fn wait(runner: &mut Runner, runs: usize) -> Vec<Reply> {
        let mut done = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while done.len() < runs && Instant::now() < deadline {
            for reply in runner.poll() {
                if matches!(reply, Reply::Done { .. }) {
                    done.push(reply);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        done
    }

    fn finished(reply: &Reply) -> &AnalysisResult {
        match reply {
            Reply::Done {
                outcome: Outcome::Finished(r),
                ..
            } => r,
            other => panic!("not finished: {other:?}"),
        }
    }

    #[test]
    fn the_rc_filter_has_the_analytic_response() {
        let c = Circuit::sample(Sample::RcLowPass);
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        let ac = runner
            .submit(1, 0, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        assert!(runner.busy());
        let replies = wait(&mut runner, 1);
        assert!(!runner.busy());
        let Reply::Done {
            run,
            circuit,
            index,
            ..
        } = &replies[0]
        else {
            panic!()
        };
        assert_eq!((*run, *circuit, *index), (ac, 1, 0));
        let result = finished(&replies[0]);
        assert_eq!(result.kind, Kind::Ac);
        let out = result.table.signal("V(out)").unwrap();
        // |H| = 1 / sqrt(1 + (2 pi f R C)^2). Closed form, so a tight tolerance: 1e-6.
        let (r, cap) = (1e3_f64, 100e-9_f64);
        for (i, f) in result.table.axis.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * f * r * cap;
            let want = 1.0 / (1.0 + w * w).sqrt();
            let got = out.magnitude(i).unwrap();
            assert!((got - want).abs() < 1e-6, "{f} Hz: {got} vs {want}");
            let phase = out.phase_deg(i).unwrap();
            assert!(
                (phase + w.atan().to_degrees()).abs() < 1e-4,
                "{f} Hz phase {phase}"
            );
        }
    }

    #[test]
    fn the_step_response_charges_like_one_minus_e_to_the_minus_t_over_tau() {
        let c = Circuit::sample(Sample::RcLowPass);
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        runner
            .submit(1, 1, netlist.clone(), netlist.analyses[1].clone())
            .unwrap();
        let replies = wait(&mut runner, 1);
        let result = finished(&replies[0]);
        let out = result.table.signal("V(out)").unwrap();
        let tau = 100e-6;
        let mut checked = 0;
        for (i, t) in result.table.axis.iter().enumerate() {
            if *t < 20e-6 {
                continue;
            }
            let want = 1.0 - (-t / tau).exp();
            let got = out.real(i).unwrap();
            // The source rises in 1 ns and the integration error is about 1e-3: 0.5 % of full scale.
            assert!((got - want).abs() < 5e-3, "t = {t}: {got} vs {want}");
            checked += 1;
        }
        assert!(checked > 20);
    }

    #[test]
    fn the_divider_has_its_operating_point() {
        let c = Circuit::sample(Sample::Divider);
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        runner
            .submit(1, 0, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        let replies = wait(&mut runner, 1);
        let result = finished(&replies[0]);
        let out = result.table.signal("V(out)").unwrap().real(0).unwrap();
        assert!((out - 10.0 * 2.2 / 3.2).abs() < 1e-9, "{out}");
    }

    #[test]
    fn a_circuit_the_engine_rejects_fails_with_its_message() {
        let c = Circuit::from_text(
            Some(std::path::Path::new("float.cir")),
            "t\nV1 a b 1\n.op\n.end\n",
        )
        .unwrap();
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        runner
            .submit(1, 0, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        let replies = wait(&mut runner, 1);
        match &replies[0] {
            Reply::Done {
                outcome: Outcome::Failed(m),
                ..
            } => assert!(m.contains("ground"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stopping_cancels_what_has_not_started_and_the_runner_goes_on() {
        let c = Circuit::sample(Sample::RcLowPass);
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        // Hold the worker busy with a long transient, queue two more, then stop all three.
        let long = Analysis::Tran {
            tstep: 1e-9,
            tstop: 5e-3,
            tstart: 0.0,
            tmax: Some(1e-9),
            uic: false,
        };
        runner.submit(1, 0, netlist.clone(), long).unwrap();
        runner
            .submit(1, 1, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        runner
            .submit(1, 2, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        runner.stop();
        let replies = wait(&mut runner, 3);
        assert_eq!(replies.len(), 3);
        for r in &replies {
            let Reply::Done { outcome, .. } = r else {
                panic!()
            };
            assert!(
                matches!(outcome, Outcome::Cancelled | Outcome::Failed(_)),
                "the stop did not take: {outcome:?}"
            );
        }
        assert!(!runner.busy());
        // It still works afterwards.
        runner
            .submit(1, 0, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        let again = wait(&mut runner, 1);
        assert!(matches!(
            again[0],
            Reply::Done {
                outcome: Outcome::Finished(_),
                ..
            }
        ));
    }

    #[test]
    fn progress_is_a_fraction_and_unknown_runs_have_none() {
        let mut runner = Runner::spawn(|| {}).unwrap();
        assert_eq!(runner.progress(42), None);
        let c = Circuit::sample(Sample::RcLowPass);
        let netlist = c.netlist.clone().unwrap();
        let run = runner
            .submit(1, 0, netlist.clone(), netlist.analyses[0].clone())
            .unwrap();
        if let Some(p) = runner.progress(run) {
            assert!((0.0..=1.0).contains(&p));
        }
        wait(&mut runner, 1);
        assert_eq!(runner.progress(run), None);
    }
}
