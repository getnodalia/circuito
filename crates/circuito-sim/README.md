# circuito-sim

Circuito's circuit simulator. It takes a `circuito_core::Netlist` and runs its analysis cards
(`.op`, `.dc`, `.ac`, `.tran`) with modified nodal analysis and Newton-Raphson iteration. The dense
LU factorisation (real and complex) comes from the shared `nodalia-numerics`; nothing else in the
solver is borrowed. No UI, no I/O.

## API

```rust
use circuito_sim::{simulate, simulate_all, Options, Control};

let results = simulate_all(&netlist, &Control::new())?; // one Result per analysis card
```

- `simulate(netlist, analysis, options, control)` runs one analysis; `simulate_all` runs every card.
- `AnalysisResult` carries a `Table` of `Signal`s (`V(node)`, `I(V1)`, `I(L1)`, ...), real or
  complex (`magnitude`, `phase_deg`, `db`), plus `RunInfo`: engine version, method, tolerances,
  Newton iterations, accepted and rejected steps, which DC strategy converged (`newton`,
  `gmin stepping`, `source stepping`, `uic`) and warnings (for example unknown model parameters).
- `Control` carries a cancel flag and a progress callback; both are honoured inside the solver loops.
- Errors (`SimError`) name the problem: the elements of a voltage-source/inductor loop, the node with
  no DC path to ground, the unknown behind a singular pivot, where Newton failed and what was tried,
  the time where the step became too small. Nothing returns a plausible wrong number: a non-finite
  result is an error.

## Modelled

R, C, L, mutual inductors (`K`), independent V and I sources (DC, SIN, PULSE, PWL, EXP, AC spec),
E/G/F/H controlled sources, diode (Shockley, series resistance, depletion and diffusion charge,
breakdown), BJT (Ebers-Moll with Early effect, junction charges, RB/RC/RE), MOSFET (Shichman-Hodges
level 1 with body effect, LAMBDA, overlap and bulk capacitances), hysteretic switch, op-amp
(finite gain with output resistance and smooth saturation, or ideal). `.options` cards: RELTOL,
ABSTOL, VNTOL, GMIN, TEMP, TRTOL, CHGTOL, ITL1, ITL4, METHOD.

## Method

- Unknowns: node voltages, branch currents of V/L/E/H, internal nodes of devices with series
  resistance. System `(G + alpha*C) x = b`; alpha is 0 at DC, 1/h (backward Euler) or 2/h
  (trapezoidal).
- DC: plain Newton with junction limiting, then gmin stepping, then source stepping.
- Transient: backward Euler on the first step and after breakpoints and switch events, then
  trapezoidal; step size from the local truncation error (charge divided differences, TRTOL 7);
  source breakpoints are hit exactly; switch events are bisected. `uic` starts from the stated
  initial conditions.
- AC: DC operating point, linearisation, one complex solve per frequency. Needs a source with an
  AC value.
- Topology is checked before solving (voltage loops, floating nodes, missing ground).
- Limits: at most 2500 unknowns, at most `max_points` (2,000,000) transient points and 40 M samples
  per run; larger runs are refused with `TooLarge` instead of attempted.

## Verified

All tests run with no network or display; `cargo test -p circuito-sim` is 62 tests (12 unit, 21 DC,
9 AC, 14 transient, 6 robustness). Tolerances are in the tests and are tight (`reltol 1e-9`,
`gmin 1e-15`) where a closed form exists.

- DC against closed forms: dividers, a Wheatstone bridge, the four controlled sources, a diode with
  series resistance against the Shockley equation solved by bisection, a zener clamp, Ebers-Moll with
  Early effect (active, saturation and the PNP mirror), the MOSFET square law and triode region,
  PMOS and the body effect, a finite-gain op-amp including its output resistance, switch hysteresis,
  diode temperature coefficient (about -2 mV/K).
- AC against transfer functions: RC low-pass, series RLC resonance and Q, a transformer with mutual
  inductance (two-port equations), the diode and BJT small-signal gain derived from the model
  equations, and the pole added by a junction capacitance.
- Transient against closed forms: RC step, RL step, underdamped series RLC, LC tank amplitude, a sine
  through an RC filter against the AC analysis, varactor charge balance, breakpoint hits. Measured
  order of accuracy with fixed steps: trapezoidal error ratio 3.2 to 4.8 per halving (second order),
  backward Euler 1.7 to 2.3 (first order). Default tolerances give an RC step error under 1e-3 of the
  step; RELTOL 1e-6 gives under 1e-4 and more steps.
- Robustness: truncated and random netlists (property tests) never panic and never return
  non-finite numbers; extreme but legal values; oversized circuits refused; cancellation and
  monotonic progress; forced fallbacks (ITL1=5 makes Newton fail and gmin or source stepping take
  over).

## Not implemented

`.noise`, `.tf`, `.step`, Monte Carlo, `.ic` / `.nodeset` cards, the Gummel-Poon extras (high
injection, Kirk effect), Meyer gate capacitances and bulk junction diodes in the MOSFET, temperature
scaling of capacitances and junction potentials. Convergence is judged on the solution vector only.
Transient output is at the solver's own accepted time points, not resampled to `tstep`. Not compared
against another simulator's output; the references are analytic solutions and the model equations
of Nagel (SPICE2, ERL-M520), Vladimirescu (*The SPICE Book*), Kundert (*The Designer's Guide to
SPICE and Spectre*) and Shichman-Hodges (1968).

## Dependency note

`nodalia-numerics` is a git dependency on getnodalia/abaco, pinned by revision.
