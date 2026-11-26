# Architecture

```text
nodalia-numerics (from Ábaco)
        |
 circuito-core --- circuito-spice
     |   \             |
     |    circuito-sim |
 circuito-erc     circuito-script (text <-> schematic)
     |                 |
 circuito-pcb (own model)      front ends: circuito-cli, egui app
```

* Dependencies point down; engines never use egui, eframe or winit.
* A schematic is converted to a netlist by `circuito-erc::extract`. Parts that cannot be converted are
  listed as issues, never dropped; `sim` refuses to run a schematic with unconverted parts because it
  would simulate a different circuit.
* The simulator reports singular systems, loops of voltage sources, nodes without a DC path and
  non-convergence as errors that name the elements; it never returns a non-finite number.
* Nothing here may drive real equipment: any future connector to hardware is opt-in, explicit and
  isolated from the simulator.
