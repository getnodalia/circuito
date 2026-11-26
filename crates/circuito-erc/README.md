# circuito-erc

Netlist extraction from a Circuito schematic, and the electrical rules check on top of it.

## API

- `extract(&Schematic) -> Result<Extraction, SchematicError>`: resolves nets, names them (label,
  power-port or automatic `N$001`-style names; ground is `0`), and builds a `circuito_core::netlist::Netlist` with
  the schematic's models and analyses carried over. Parts that cannot become elements (no value,
  unreadable value, unknown model, duplicate reference) are not dropped silently: they come back
  in `Extraction::issues` with the reference designator and a reason. `default_models()` supplies
  the generic models of the built-in library.
- `check(&Schematic) -> ErcReport`: never fails; every finding is a `Violation` with a stable code,
  `Severity` (error, warning, info), sheet, position and the items involved. `ErcReport::to_json`.
- `conflict(a, b) -> Verdict`: the pin-type matrix (two push-pull drivers fight, open-collector
  outputs may share a net, an output into a power input is only suspicious, a no-connect pin
  conflicts with everything).

## Rules

E001 pin unconnected, E002 single-pin net, E003 connected no-connect, E010 output conflict, E011
power-output conflict, E012 other pin-type conflict, E020 undriven power input, E021 no ground,
E030 duplicate reference, E031 missing value, E032 invalid value, E033 unknown model, E034 unknown
symbol, E035 part rejected, E040 floating input, E050 voltage-source loop, E051 current-source
cut-set, E052 no DC path to ground, E060 net-name conflict, E070 touch without junction, E071
floating label, E072 ambiguous label, E073 floating power port, E074 dangling junction, E075
dangling no-connect. Codes never change meaning.

## Verified

`cargo test -p circuito-erc`: 9 unit and 23 integration tests. Every rule has a schematic that must
fire it exactly once, and clean schematics (divider, RC low-pass, ...) must stay silent. Extraction
is checked for deterministic output and for netlists that validate and print as expected
(`V1 IN 0 DC 10`, `R1 IN OUT 1k`).

## Limits

The checks are structural and electrical-topological; they do not simulate. E050 to E052 are
conservative graph checks (they are exact for ideal sources and DC paths; they do not know about
parasitics a simulation would add). Pin types come from the built-in library only.
