# Circuito roadmap

Only what runs is described as existing. Levels: planned, experimental, partially functional,
functional, validated (against analytic or published references), production-mature.

| Area | State |
|---|---|
| Values, netlist, schematic model, part library | Functional |
| SPICE reader/writer | Functional for the supported cards; others are refused with a line number |
| DC operating point | Validated against closed forms (dividers, bridge, controlled sources, diode, BJT, MOSFET, op-amp) |
| AC analysis | Validated against transfer functions (RC, series RLC, transformer, small-signal gain) |
| Transient | Validated against closed forms (RC, RL, RLC, LC); measured order 2 (trapezoidal) and 1 (backward Euler) |
| ERC | Functional, 25 rules |
| PCB model and DRC | Functional; boundary-tested at the rule value and 1 nm below |
| Gerber and Excellon output | Experimental: round-trips through our own readers; not opened in an external CAM viewer |
| CLI: `sim`, `erc`, `netlist` | Functional on Linux |
| CLI: `drc`, `gerber` | Planned (engines exist) |
| egui app, schematic editor, PCB editor | Planned |
| `.noise`, `.step`, Monte Carlo, `.ic`/`.nodeset` | Planned |
| Zone fill, autorouting, differential-pair rules | Planned |
| Comparison with another simulator's output | Not done |
| PLC, industrial electrical, FPGA, embedded | Planned |

## Next

1. `drc` and `gerber` commands, and a sample board.
2. The egui app with NDL: schematic view, simulation panel with plots, ERC list.
3. Validate Gerber output with an external viewer and publish the result here.
4. Simulation: `.step`, `.noise`, Gummel-Poon, temperature scaling.
5. PCB editor, zone fill, routing aids.

## Known limitations

See the README of each crate; the important ones: convergence is judged on the solution vector
only; transient output is at the solver's accepted time points; the DRC approximates arcs to 1 µm;
no hierarchical sheets or buses.
