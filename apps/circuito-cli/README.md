# circuito (command line)

The headless `circuito` command. Pure Rust, no display needed.

```text
circuito sim <file> [--format table|csv|json] [-o <out>] [-a <n>]
circuito erc <file.circuito> [--json] [--deny-warnings]
circuito netlist <file.circuito> [-o <out>]
```

- `sim` runs the analysis cards (`.op .dc .ac .tran`) of a SPICE netlist or a `.circuito`
  schematic (chosen by the `.circuito` extension or a leading `{`). `table` prints aligned columns
  (`.op` one signal per line), `csv` and `json` are for scripts; AC results print magnitude, dB and
  phase per signal. Numbers have nine significant digits. `-a N` runs only the N-th card; one
  failing card does not hide the others (they are printed, the failure goes to standard error, the
  exit code is 1). The header of each result records the engine, how the operating point was found
  and the integration method.
- `erc` checks a schematic. Exit 1 if there are errors (or warnings with `--deny-warnings`).
- `netlist` writes the SPICE text of a schematic. Parts that could not be extracted are listed on
  standard error and make the exit code 1 (the rest is still written); `sim` refuses such a
  schematic outright, because it would simulate a different circuit.
- `-` as a file name reads standard input; `-o` writes through a temporary file and renames it.

Exit codes: 0 success, 1 the input or an analysis failed, 2 bad command line. Input is limited to
64 MiB and must be UTF-8; nothing panics on malformed input.

Examples are in `/examples`: `rc-lowpass.cir`, `diode-rectifier.cir`, `series-rlc.cir`,
`opamp-amplifier.cir`, `divider.circuito`.

## Verified

`cargo test -p circuito-cli` runs the real binary: the RC AC response against `1/sqrt(1+(wRC)^2)`
and its phase at all 41 frequencies (1e-6), the RC step response against `1-exp(-t/RC)` (2e-3 of
the step), the series-RLC response against its closed form at every frequency (1e-6) and its peak
against `Q`, the inverting op-amp against the exact finite-gain solution (1e-6) and its saturation,
the divider against 10 * 2.2/3.2 (1e-9), a rectifier's peak and hold, the schematic-to-SPICE round
trip, error messages (line numbers, loop elements, missing analysis, bad index), exit codes, and
hostile inputs (binary, truncated JSON, wrong versions, huge values) without a crash.

## Not here yet

PCB commands (`drc`, `gerber`) belong to `circuito-pcb` and are not wired in. No `.noise`, `.step`
or measurement cards, no waveform viewer.
