# circuito-spice

A SPICE netlist reader and writer for `circuito_core::netlist::Netlist`.

## API

- `parse(&str) -> Result<Parsed, ParseError>`: `Parsed` has the netlist and `warnings`; a
  `ParseError` carries the 1-based line and a message that says what to do.
- `write(&Netlist) -> Result<String, NetlistError>`: deterministic SPICE text that `parse` reads
  back to an equal netlist.
- `expr`: the `{expression}` evaluator for `.param` (arithmetic, parameters, common functions).

## Supported

Title line, comments (`*`, inline `;` and ` $`), continuation lines (`+`), case-insensitive input,
SI suffixes, `.param` with `{expr}`, `.model` (D, NPN, PNP, NMOS, PMOS, SW, OPAMP), `.subckt` /
`.ends` (no parameters, no nesting), `.options`, `.temp`, `.op`, `.dc`, `.ac`, `.tran`, `.end`
(text after it is ignored), elements R C L V I D E G F H K S Q M U X, sources with DC, AC, SIN,
PULSE, PWL and EXP.

`.print .plot .probe .save .width .nodeset .meas .measure` are ignored with a warning.
`.include .lib .global .ic .noise .step .tf .four` and anything else are errors, never silently
skipped. A netlist can never make the reader open a file. Input is limited to 16 MiB and 64 KiB per
logical line.

## Verified

`cargo test -p circuito-spice`: 3 unit and 13 integration tests. A netlist using every card parses
to the expected structure; write-then-parse is the identity (also for flattened netlists and for
generated simple netlists); line numbers on errors; `.include` refused without reading anything;
ignored commands warn; arbitrary text, random cards, truncations and byte mutations of a full
netlist never panic.

## Not done

Parameterised subcircuits, nested `.subckt` definitions, `.ic` and `.nodeset`, `.step`, behavioural
sources (`B`), transmission lines, and vendor model dialects. A deck that uses them fails with the
line and the reason.
