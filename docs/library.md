# Built-in part library

Every symbol in `crates/circuito-core/src/library.rs` is **original work** drawn for Circuito.
The shapes follow textbook schematic conventions (a box for a resistor, two plates for a
capacitor, a triangle and a bar for a diode, a circle with a line for a transistor); no other
tool's symbol files, libraries or artwork were opened, traced or copied. The geometry is data
(`Shape` values on an integer grid of 1.27 mm), so it is also what the application draws.

Parts: R, C, C_POL, L, V, I, D, LED, ZENER, NPN, PNP, NMOS, PMOS, OPAMP, OPAMP_PWR, SWITCH, POT,
TRANSFORMER, FUSE, and the power symbols GND and VCC.

The default simulation models (`D`, `LED`, `DZ`, `NPN`, `PNP`, `NMOS`, `PMOS`, `SW`, `OPAMP`) are
generic, textbook-typical parameter sets chosen for plausible behaviour (for example the LED model
gives about 1.9 V at 10 mA). They are not the parameters of any manufacturer's part; replace them
with a `.model` card for the real device.

The part list in the editor (`apps/circuito/src/parts.rs`) groups these by use; a test checks that
every part of the library is offered and every entry exists.
