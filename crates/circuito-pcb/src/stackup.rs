// SPDX-License-Identifier: MIT OR Apache-2.0
//! Board stackup: copper and dielectric layers with thickness, permittivity and copper weight.

use serde::{Deserialize, Serialize};

use crate::error::{PcbError, Result};
use crate::layer::Layer;
use crate::units::Nm;

/// 1 oz/ft² of copper is 34.8 µm thick (rounded to 0.1 µm).
pub const NM_PER_OZ: Nm = 34_800;

/// One layer of the physical stack, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StackLayer {
    Copper {
        layer: Layer,
        thickness: Nm,
        /// Copper weight in thousandths of an ounce per square foot (1 oz = 1000).
        weight_milli_oz: u32,
    },
    Dielectric {
        thickness: Nm,
        /// Relative permittivity in thousandths (FR-4 is about 4500).
        epsilon_r_milli: u32,
        /// Free-form material name.
        material: String,
    },
}

/// The ordered physical layers of the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stackup {
    pub layers: Vec<StackLayer>,
}

impl Stackup {
    /// A conventional 1.6 mm FR-4 stackup: 1 oz outer copper, 0.5 oz inner copper.
    pub fn standard(copper_layers: u8) -> Stackup {
        let stack = Layer::copper_stack(copper_layers);
        let n = stack.len();
        let total: Nm = 1_600_000;
        let mut layers = Vec::new();
        let copper_total: Nm = stack
            .iter()
            .enumerate()
            .map(|(i, _)| {
                if i == 0 || i + 1 == n {
                    NM_PER_OZ
                } else {
                    NM_PER_OZ / 2
                }
            })
            .sum();
        let gaps = if n > 1 { n - 1 } else { 1 };
        let diel = (total - copper_total) / gaps as Nm;
        for (i, l) in stack.iter().enumerate() {
            let outer = i == 0 || i + 1 == n;
            layers.push(StackLayer::Copper {
                layer: *l,
                thickness: if outer { NM_PER_OZ } else { NM_PER_OZ / 2 },
                weight_milli_oz: if outer { 1000 } else { 500 },
            });
            if i + 1 < n || n == 1 {
                layers.push(StackLayer::Dielectric {
                    thickness: diel,
                    epsilon_r_milli: 4500,
                    material: "FR-4".into(),
                });
            }
        }
        Stackup { layers }
    }

    /// Total thickness in nm (checked).
    pub fn total_thickness(&self) -> Result<Nm> {
        let mut t: Nm = 0;
        for l in &self.layers {
            let x = match l {
                StackLayer::Copper { thickness, .. } | StackLayer::Dielectric { thickness, .. } => {
                    *thickness
                }
            };
            t = t
                .checked_add(x)
                .ok_or(PcbError::Overflow("stackup thickness"))?;
        }
        Ok(t)
    }

    /// Validates the stackup against a board with `copper_layers` copper layers.
    pub fn validate(&self, copper_layers: u8) -> Result<()> {
        let bad = |d: String| PcbError::Invalid(format!("stackup: {d}"));
        if self.layers.len() > 2 * crate::layer::MAX_COPPER_LAYERS as usize {
            return Err(bad("too many layers".into()));
        }
        let expect = Layer::copper_stack(copper_layers);
        let mut copper_seen = 0usize;
        for (i, l) in self.layers.iter().enumerate() {
            let want_copper = i % 2 == 0;
            match l {
                StackLayer::Copper {
                    layer,
                    thickness,
                    weight_milli_oz,
                } => {
                    if !want_copper {
                        return Err(bad(format!(
                            "entry {i} is copper but a dielectric must separate copper layers"
                        )));
                    }
                    if expect.get(copper_seen) != Some(layer) {
                        return Err(bad(format!("copper entry {copper_seen} is {} but the board's copper layer {copper_seen} is {}", layer, expect.get(copper_seen).map(|l| l.name()).unwrap_or_else(|| "absent".into()))));
                    }
                    copper_seen += 1;
                    if !(1_000..=500_000).contains(thickness) {
                        return Err(bad(format!(
                            "{layer} thickness {thickness} nm is outside 1 µm..500 µm"
                        )));
                    }
                    if !(100..=10_000).contains(weight_milli_oz) {
                        return Err(bad(format!(
                            "{layer} copper weight {weight_milli_oz}/1000 oz is outside 0.1..10 oz"
                        )));
                    }
                    // thickness must agree with the weight within 30 %
                    let nominal = (*weight_milli_oz as i128) * NM_PER_OZ as i128 / 1000;
                    let diff = (*thickness as i128 - nominal).abs();
                    if diff * 100 > nominal * 30 {
                        return Err(bad(format!("{layer} thickness {thickness} nm disagrees with its weight of {} oz (about {nominal} nm)", *weight_milli_oz as f64 / 1000.0)));
                    }
                }
                StackLayer::Dielectric {
                    thickness,
                    epsilon_r_milli,
                    material,
                } => {
                    if want_copper {
                        return Err(bad(format!(
                            "entry {i} is a dielectric but copper is expected here"
                        )));
                    }
                    if !(5_000..=5_000_000).contains(thickness) {
                        return Err(bad(format!(
                            "dielectric {i} thickness {thickness} nm is outside 5 µm..5 mm"
                        )));
                    }
                    if !(1_000..=100_000).contains(epsilon_r_milli) {
                        return Err(bad(format!(
                            "dielectric {i} permittivity {epsilon_r_milli}/1000 is outside 1..100"
                        )));
                    }
                    if material.len() > 128 || material.chars().any(char::is_control) {
                        return Err(bad(format!("dielectric {i} material name is invalid")));
                    }
                }
            }
        }
        if copper_seen != expect.len() {
            return Err(bad(format!(
                "has {copper_seen} copper layers but the board has {}",
                expect.len()
            )));
        }
        let total = self.total_thickness()?;
        if !(20_000..=10_000_000).contains(&total) {
            return Err(bad(format!(
                "total thickness {total} nm is outside 20 µm..10 mm"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_stackups_validate_and_are_about_1_6_mm() {
        for n in [1u8, 2, 4, 6, 8, 32] {
            let s = Stackup::standard(n);
            s.validate(n).unwrap();
            let t = s.total_thickness().unwrap();
            assert!((1_500_000..=1_700_000).contains(&t), "{n} layers: {t}");
        }
    }

    #[test]
    fn rejects_inconsistent_stackups() {
        let mut s = Stackup::standard(2);
        assert!(s.validate(4).is_err());
        s.layers.swap(0, 1);
        assert!(s.validate(2).is_err());
        let mut s = Stackup::standard(2);
        if let Some(StackLayer::Copper { thickness, .. }) = s.layers.get_mut(0) {
            *thickness = 100_000;
        }
        assert!(s.validate(2).is_err());
        let mut s = Stackup::standard(2);
        if let Some(StackLayer::Dielectric {
            epsilon_r_milli, ..
        }) = s.layers.get_mut(1)
        {
            *epsilon_r_milli = 0;
        }
        assert!(s.validate(2).is_err());
        let s = Stackup { layers: vec![] };
        assert!(s.validate(2).is_err());
    }
}
