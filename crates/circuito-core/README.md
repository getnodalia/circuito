# circuito-core

The data model every Circuito engine and app shares: engineering values, source waveforms, the
netlist, the schematic with its net resolution, and the built-in part library. Pure data and
checks; no I/O, no solver.

## API

- `parse_value` / `format_value`: engineering notation (`4.7k`, `100n`, `1meg`, `2R2`, `1u5`),
  trailing units ignored (`10uF`), exact round trip for any finite value.
- `source::Source` and `Waveform` (DC, SIN, PULSE, PWL, EXP, plus an AC magnitude and phase):
  `dc_value`, `value_at(t)`, `breakpoints`, a `Display` that parses back to the same source.
- `netlist::Netlist`: nodes (case-insensitive), elements R C L V I D E G F H K S Q M U X, `.model`
  cards, `.subckt` definitions, analyses (`Op`, `Dc`, `Ac`, `Tran`), `.param` and `.options`
  values; `validate()` (duplicates, dangling controlling elements, model kinds, analysis ranges),
  `flatten()` (subcircuit expansion, recursion refused) and a deterministic `Display` in SPICE
  syntax.
- `schematic::Schematic`: sheets of parts, wires, junctions, labels, power ports and no-connects,
  plus the models and analyses that go with it. `.circuito` is its JSON text
  (`to_json`/`from_json`, `format_version` 1, other versions refused with a hint).
  `schematic::nets` resolves connectivity (wire ends, pins on wire interiors need a junction,
  crossing wires do not connect, labels by name per sheet, global power ports, ground aliases).
  `schematic::builder::Builder` assembles a schematic programmatically.
- `library`: 19 parts (R, C, C_POL, L, V, I, D, LED, ZENER, NPN, PNP, NMOS, PMOS, OPAMP,
  OPAMP_PWR, SWITCH, POT, TRANSFORMER, FUSE) plus the GND and VCC power symbols, with pin
  geometry, rotation and mirroring, and generic default models. The symbols are original work; see
  `docs/library.md`.

## Limits

A schematic file is read only up to 64 MiB, 256 sheets, 200,000 parts and 500,000 wires per
sheet, 10,000 vertices per wire, 1,024 characters of text, coordinates within +-10,000,000.
Anything larger is an error naming the limit. Nothing here panics on input.

## Verified

`cargo test -p circuito-core`: 45 tests. Value parsing and formatting (suffix table, `R` notation,
garbage rejection, exact round trip for arbitrary finite floats, never panics: property tests);
source parsing, waveform values, breakpoints and display round trip; netlist validation and
flattening (nested subcircuits, recursion and bad instances rejected); net resolution case by case
(junction rules, crossing wires, rotation and mirroring moving pins, hostile coordinates);
schematic JSON round trip, truncated and oversize input.

## Not done

Hierarchical sheets and buses are not modelled (sheets are joined by global labels and power ports
only). The default part models are generic textbook parameter sets, not any manufacturer's part.
