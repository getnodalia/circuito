// SPDX-License-Identifier: MIT OR Apache-2.0
//! Solver options and the cancel / progress hooks.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use circuito_core::netlist::Netlist;

use crate::error::SimError;

/// The implicit integration method for transient analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integration {
    /// Trapezoidal rule (second order); backward Euler for the first step and after every
    /// source breakpoint, to avoid trapezoidal ringing. The default.
    Trapezoidal,
    /// Backward Euler (first order, strongly damped) throughout.
    BackwardEuler,
}

/// Solver tolerances and limits. The names follow the SPICE `.options` cards.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Relative tolerance on unknowns and on the local truncation error (`RELTOL`, 1e-3).
    pub reltol: f64,
    /// Absolute tolerance on branch currents, amperes (`ABSTOL`, 1e-12).
    pub abstol: f64,
    /// Absolute tolerance on node voltages, volts (`VNTOL`, 1e-6).
    pub vntol: f64,
    /// Conductance added across every semiconductor junction and drain-source (`GMIN`, 1e-12).
    pub gmin: f64,
    /// Circuit temperature in degrees Celsius (`TEMP`, 27). Model parameters are given at 27.
    pub temp: f64,
    /// Newton iteration limit for a DC solution (`ITL1`, 100).
    pub itl1: usize,
    /// Newton iteration limit per transient time point (`ITL4`, 10).
    pub itl4: usize,
    /// Divisor applied to the local-truncation-error estimate to compensate for its
    /// conservatism (`TRTOL`, 7).
    pub trtol: f64,
    /// Absolute charge tolerance, coulombs or webers (`CHGTOL`, 1e-14).
    pub chgtol: f64,
    /// The integration method (`METHOD`: 0 trapezoidal, 1 backward Euler).
    pub method: Integration,
    /// The most time points a transient run may store (default 2,000,000).
    pub max_points: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            reltol: 1e-3,
            abstol: 1e-12,
            vntol: 1e-6,
            gmin: 1e-12,
            temp: 27.0,
            itl1: 100,
            itl4: 10,
            trtol: 7.0,
            chgtol: 1e-14,
            method: Integration::Trapezoidal,
            max_points: 2_000_000,
        }
    }
}

impl Options {
    /// The options a netlist's `.options` cards ask for, on top of the defaults. Names the
    /// simulator does not know are ignored.
    ///
    /// # Errors
    /// A value outside its legal range.
    pub fn from_netlist(n: &Netlist) -> Result<Options, SimError> {
        let mut o = Options::default();
        let get = |k: &str| n.options.get(k).copied();
        if let Some(v) = get("RELTOL") {
            o.reltol = v;
        }
        if let Some(v) = get("ABSTOL") {
            o.abstol = v;
        }
        if let Some(v) = get("VNTOL") {
            o.vntol = v;
        }
        if let Some(v) = get("GMIN") {
            o.gmin = v;
        }
        if let Some(v) = get("TEMP") {
            o.temp = v;
        }
        if let Some(v) = get("TRTOL") {
            o.trtol = v;
        }
        if let Some(v) = get("CHGTOL") {
            o.chgtol = v;
        }
        if let Some(v) = get("ITL1") {
            o.itl1 = whole(v, "ITL1")?;
        }
        if let Some(v) = get("ITL4") {
            o.itl4 = whole(v, "ITL4")?;
        }
        if let Some(v) = get("METHOD") {
            o.method = if v == 0.0 {
                Integration::Trapezoidal
            } else if v == 1.0 {
                Integration::BackwardEuler
            } else {
                return Err(SimError::Invalid(format!(
                    "METHOD must be 0 (trapezoidal) or 1 (backward Euler), got {v}"
                )));
            };
        }
        o.validate()?;
        Ok(o)
    }

    /// Check every field against its legal range.
    ///
    /// # Errors
    /// The first field out of range.
    pub fn validate(&self) -> Result<(), SimError> {
        let pos = |v: f64, name: &str| {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                Err(SimError::Invalid(format!(
                    "{name} must be a positive number, got {v}"
                )))
            }
        };
        pos(self.reltol, "RELTOL")?;
        if self.reltol >= 1.0 {
            return Err(SimError::Invalid(format!(
                "RELTOL must be below 1, got {}",
                self.reltol
            )));
        }
        pos(self.abstol, "ABSTOL")?;
        pos(self.vntol, "VNTOL")?;
        pos(self.trtol, "TRTOL")?;
        pos(self.chgtol, "CHGTOL")?;
        if !(self.gmin.is_finite() && self.gmin >= 0.0 && self.gmin <= 1.0) {
            return Err(SimError::Invalid(format!(
                "GMIN must be between 0 and 1 siemens, got {}",
                self.gmin
            )));
        }
        if !(self.temp.is_finite() && self.temp > -273.0 && self.temp < 1000.0) {
            return Err(SimError::Invalid(format!(
                "TEMP must be between -273 and 1000 degrees Celsius, got {}",
                self.temp
            )));
        }
        if self.itl1 < 5 || self.itl4 < 3 {
            return Err(SimError::Invalid(
                "ITL1 must be at least 5 and ITL4 at least 3".into(),
            ));
        }
        if self.max_points < 10 {
            return Err(SimError::Invalid("max_points must be at least 10".into()));
        }
        Ok(())
    }
}

fn whole(v: f64, name: &str) -> Result<usize, SimError> {
    if (0.0..=1.0e7).contains(&v) && v.fract() == 0.0 {
        Ok(v as usize)
    } else {
        Err(SimError::Invalid(format!(
            "{name} must be a whole number up to 10,000,000, got {v}"
        )))
    }
}

/// Hooks a caller can give a running analysis: a cancel flag checked between Newton iterations
/// and time points, and a progress callback receiving a fraction between 0 and 1.
#[derive(Clone, Default)]
pub struct Control {
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Arc<dyn Fn(f64) + Send + Sync>>,
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Control")
            .field("cancel", &self.cancel.is_some())
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl Control {
    /// No hooks.
    #[must_use]
    pub fn new() -> Self {
        Control::default()
    }

    /// Stop the analysis with [`SimError::Cancelled`] once `flag` becomes true.
    #[must_use]
    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    /// Call `f` with the completed fraction as the analysis advances.
    #[must_use]
    pub fn with_progress(mut self, f: Arc<dyn Fn(f64) + Send + Sync>) -> Self {
        self.progress = Some(f);
        self
    }

    pub(crate) fn check(&self) -> Result<(), SimError> {
        match &self.cancel {
            Some(f) if f.load(Ordering::Relaxed) => Err(SimError::Cancelled),
            _ => Ok(()),
        }
    }

    pub(crate) fn report(&self, fraction: f64) {
        if let Some(p) = &self.progress {
            p(fraction.clamp(0.0, 1.0));
        }
    }
}
