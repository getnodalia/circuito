// SPDX-License-Identifier: MIT OR Apache-2.0
//! The pin-type conflict matrix: what happens when two pins of given types share a net.
//!
//! This matrix is Circuito's own. It encodes ordinary electrical practice:
//!
//! * Two push-pull drivers on one net fight (`Output`/`Output`, `Output`/`PowerOut`,
//!   `PowerOut`/`PowerOut`, `Output`/`OpenCollector`, `PowerOut`/`OpenCollector`): error.
//! * Open-collector outputs may share a net (wired-OR): fine.
//! * A bidirectional pin may drive or listen, so meeting a driver is only suspicious: warning.
//! * An output feeding a power input is unusual: warning.
//! * A pin marked `Nc` conflicts with every other pin: error.
//! * Passive and `Unconnected` pins never conflict.

use circuito_core::schematic::PinType;

/// How serious a pairing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Fine.
    Ok,
    /// Suspicious.
    Warning,
    /// A real conflict.
    Error,
}

/// The verdict for two pin types on the same net. Symmetric.
#[must_use]
pub fn conflict(a: PinType, b: PinType) -> Verdict {
    use PinType::{Bidirectional as Bi, Nc, OpenCollector as Oc, Output as Out, PowerIn, PowerOut};
    use Verdict::{Error, Warning};
    if a == Nc || b == Nc {
        return if a == b { Verdict::Ok } else { Error };
    }
    // Every non-OK pairing; anything not listed is fine.
    const TABLE: [(PinType, PinType, Verdict); 8] = [
        (Out, Out, Error),
        (Out, PowerOut, Error),
        (Out, Oc, Error),
        (PowerOut, PowerOut, Error),
        (PowerOut, Oc, Error),
        (Out, Bi, Warning),
        (PowerOut, Bi, Warning),
        (Out, PowerIn, Warning),
    ];
    TABLE
        .iter()
        .find(|(x, y, _)| (*x == a && *y == b) || (*x == b && *y == a))
        .map_or(Verdict::Ok, |t| t.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PinType::*;

    const ALL: [PinType; 9] = [
        Passive,
        Input,
        Output,
        Bidirectional,
        PowerIn,
        PowerOut,
        OpenCollector,
        Unconnected,
        Nc,
    ];

    #[test]
    fn symmetric() {
        for a in ALL {
            for b in ALL {
                assert_eq!(conflict(a, b), conflict(b, a), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn documented_cases() {
        assert_eq!(conflict(Output, Output), Verdict::Error);
        assert_eq!(conflict(PowerOut, PowerOut), Verdict::Error);
        assert_eq!(conflict(Output, PowerOut), Verdict::Error);
        assert_eq!(conflict(OpenCollector, OpenCollector), Verdict::Ok);
        assert_eq!(conflict(Output, OpenCollector), Verdict::Error);
        assert_eq!(conflict(Output, Input), Verdict::Ok);
        assert_eq!(conflict(Output, Bidirectional), Verdict::Warning);
        assert_eq!(conflict(Output, PowerIn), Verdict::Warning);
        assert_eq!(conflict(PowerOut, PowerIn), Verdict::Ok);
        assert_eq!(conflict(Passive, Output), Verdict::Ok);
        assert_eq!(conflict(Passive, Passive), Verdict::Ok);
        assert_eq!(conflict(Nc, Passive), Verdict::Error);
        assert_eq!(conflict(Nc, Nc), Verdict::Ok);
        assert_eq!(conflict(Unconnected, Output), Verdict::Ok);
        assert_eq!(conflict(Bidirectional, Bidirectional), Verdict::Ok);
        assert_eq!(conflict(PowerOut, Bidirectional), Verdict::Warning);
        assert_eq!(conflict(PowerOut, OpenCollector), Verdict::Error);
    }
}
