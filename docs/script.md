# Circuit code

Circuito can show any schematic as text and draw any text as a schematic. The two forms stay in
step: change the drawing and the code is written again; change the code and, a moment after you
stop typing, the drawing follows. The code pane is on the right of the sheet (View, Code), and a
circuit saved as `.circ` is this text.

```text
# A resistor divider. Comments start with # or //.
title Resistor divider

V1 10     in  gnd        # a part, its value, then one net per pin
R1 1k     in  out
R2 2.2k   out gnd

op                       # the analysis to run
```

## Parts

A line is `NAME [:KIND] [VALUE] NET NET …`.

* **Name:** a letter, then letters, digits or `_`. The first letters say what the part is when no
  kind is given: `R` resistor, `C` capacitor, `L` inductor, `V` voltage source, `I` current source,
  `D` diode, `LED`, `Z`/`DZ` Zener, `Q` NPN transistor, `M` N-channel MOSFET, `U` op-amp, `S`
  switch, `T` transformer, `F` fuse, `RV` potentiometer. Anything else needs a kind:
  `Q1:PNP`, `X3:OPAMP_PWR`.
* **Value:** optional, with engineering suffixes (`4.7k`, `100n`, `2u2`). A value that has spaces
  or brackets goes in quotes when it has more than one word: `V1 "SIN(0 1 1k) AC 1" in gnd`.
  Parts without a value (transistors, diodes without a model) take only nets.
* **Nets:** one per pin, in the order of the part's pins (`R`: 1, 2). A net is any word without
  spaces. `gnd` and `0` are ground; `vcc`, `vdd`, `+5v`, `-12v` and the like are drawn as supply
  symbols. `_` marks a pin as deliberately unconnected and `?` as open. Names such as `n1`, `n2`
  join pins in the text and are not drawn as labels.
* **Named pins:** write `pin=net` instead of the order, so it cannot be wrong:
  `Q1:NPN c=out b=in e=gnd`, `U1:OPAMP +=a -=b out=y`.
* **Fields:** `name=value` after the nets sets a field of the part: `ic=`, `k=`, `ratio=`,
  `rseries=` and `wiper=`; `model=NAME` picks a model card.
* **Position:** an optional `@x,y` gives where the part sits in the drawing, followed by `r90`,
  `r180`, `r270` and `mirror`. Leave it out and the part is placed and wired automatically. The
  code pane leaves positions out so the text stays short; a file saved as `.circ` writes them, so
  your layout comes back when you open it.

## Models and analyses

```text
model QFAST npn bf=200 is=1e-14      # model NAME KIND param=value …
Q1 c=out b=in e=gnd model=QFAST

op
dc V1 0 10 0.5                       # dc SOURCE START STOP STEP
ac dec 20 10 100k                    # ac dec|oct|lin POINTS FSTART FSTOP
tran 10u 5m                          # tran TSTEP TSTOP [TSTART] [TMAX] [uic]
```

Model kinds are `D`, `NPN`, `PNP`, `NMOS`, `PMOS`, `SW` and `OPAMP`. The analyses are the same
cards as in a SPICE deck, without the dot.

## How the two forms are kept in step

* Writing code never throws the drawing away. The text is compared with the drawing: parts that
  are still there keep their position, rotation and wires; new parts are placed and routed;
  parts that are gone are removed with their wires.
* While the text has a problem, the drawing keeps the last circuit that was understood and the
  problems are listed by line under the code. Nothing is half applied.
* Editing the drawing writes the code again, unless you are in the middle of typing in it.
* Undo and redo work on both: what you type in the code is merged into few undo steps, and
  undoing one puts the code back as well.
* The code form needs one sheet. A drawing with several sheets is shown as a drawing only.

## From the command line

`circuito sim`, `erc` and `netlist` accept a `.circ` file wherever they accept a `.circuito`
schematic; a mistake in the text is reported with its line.

```console
$ circuito sim examples/divider.circ
$ circuito netlist examples/common-emitter.circ
```
