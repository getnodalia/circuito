// SPDX-License-Identifier: MIT OR Apache-2.0
//! The one error type of the simulator. Every message says what is wrong and, where it can be
//! known, which element or node to look at.

use thiserror::Error;

/// Why a simulation could not run or did not finish.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SimError {
    /// The netlist itself is not valid (unknown model, bad value, ...).
    #[error("invalid netlist: {0}")]
    Netlist(String),
    /// An analysis request or an option is out of range.
    #[error("invalid request: {0}")]
    Invalid(String),
    /// A model parameter is out of its legal range.
    #[error("model {model}: {message}")]
    BadModel {
        /// The `.model` name.
        model: String,
        /// What is wrong.
        message: String,
    },
    /// Nothing in the circuit touches node 0.
    #[error("the circuit has no connection to ground (node 0 or gnd): every node voltage is relative to it")]
    NoGround,
    /// Ideal voltage sources (and inductors, which are short circuits at DC) form a loop.
    #[error("{elements:?} form a loop of ideal voltage sources{inductors}: their voltages cannot all be satisfied (add a series resistance)")]
    VoltageLoop {
        /// The elements around the loop.
        elements: Vec<String>,
        /// Extra text when inductors take part.
        inductors: String,
    },
    /// A node has no DC path to ground.
    #[error("node {node} has no DC path to ground: it touches only capacitors, current sources, controlled-source inputs or MOSFET gates (connect a resistor to ground or a bias source)")]
    NoDcPath {
        /// The node's name.
        node: String,
    },
    /// The linear system became singular.
    #[error("the equations are singular at {unknown}: {hint}")]
    Singular {
        /// The unknown whose pivot vanished.
        unknown: String,
        /// What usually causes it.
        hint: String,
    },
    /// Newton-Raphson did not converge.
    #[error("{analysis} did not converge{at}: {iterations} iterations, worst unknown {worst}; tried {tried}")]
    NoConvergence {
        /// `.op`, `.dc`, `.tran`.
        analysis: String,
        /// ` at t = ...` or an empty string.
        at: String,
        /// Iterations of the last attempt.
        iterations: usize,
        /// The unknown that moved most in the last iteration.
        worst: String,
        /// The strategies tried.
        tried: String,
    },
    /// The transient step shrank below the floor.
    #[error("transient step became too small ({step:e} s) at t = {time:e} s; the circuit may have a discontinuity it cannot resolve or an unstable loop")]
    StepTooSmall {
        /// The time reached.
        time: f64,
        /// The step that failed.
        step: f64,
    },
    /// A number went NaN or infinite.
    #[error("a non-finite number appeared in {0}")]
    NonFinite(String),
    /// A request exceeds a documented limit.
    #[error("too large: {0}")]
    TooLarge(String),
    /// A `.dc` source (or similar reference) does not exist.
    #[error("no source named {0}")]
    UnknownSource(String),
    /// The caller cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<circuito_core::netlist::NetlistError> for SimError {
    fn from(e: circuito_core::netlist::NetlistError) -> Self {
        SimError::Netlist(e.to_string())
    }
}
