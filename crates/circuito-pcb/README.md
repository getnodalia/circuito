# circuito-pcb

Circuito's PCB engine: the board model, exact integer geometry, the design rules check (DRC), and
the manufacturing outputs (Gerber RS-274X/X2, Excellon drill files, pick-and-place, bill of
materials). It is Rust only, has no dependencies beyond `thiserror`, `serde` and `serde_json`
(`proptest` for tests), and does not call any other program: the Gerber and Excellon writers and
readers are Circuito's own, written from the public specifications.

Licence: `MIT OR Apache-2.0`. This crate is a library; it has no user interface and no autorouter.

## What it does

* **Board model** (`board`, `layer`, `stackup`, `io`). Integer coordinates in nanometres, layers
  (`F.Cu`, `In1.Cu` to `In30.Cu`, `B.Cu`, mask, paste, silkscreen, courtyard, fab, edge cuts,
  drill), a stackup that is validated, nets and net classes (width, clearance, via size, differential
  pair), footprints with pads (circle, rectangle, oval, rounded rectangle, custom polygon; through
  hole with round or slot drill, SMD, non-plated), tracks (segments and arcs), vias (through, blind,
  buried, micro), zones, keepouts, an outline with cut-outs, graphics and text. Every object has a
  stable id. Boards are saved as JSON `.circuito-pcb` files with `format_version: 1`.
* **Exact geometry** (`geom`). Distances between points, segments with a width, discs and polygons
  with holes are decided in integer arithmetic (128-bit with exact 256-bit products), so a clearance
  of "at least X" is true or false with no floating-point doubt.
* **Design rules check** (`drc`). Rules plus per-net-class overrides and two built-in manufacturer
  capability profiles written as data.
* **Gerber output** (`gerber`) with a reader of its own, **Excellon output** (`excellon`) with a
  reader of its own, **pick-and-place and BOM** (`assembly`), and a tiny exact **45/90 degree
  connector** with a clearance check (`route`).

## Public API

The entry points are `Board::new`, `Board::validate`, `Board::to_json` / `Board::from_json`,
`drc::run_drc(&board, &rules)` (with `Rules`, `CapabilityProfile::generic_2layer()` /
`generic_4layer()`, `drc::check_profile`), `gerber::write_all` / `gerber::write_layer` /
`gerber::read_gerber`, `excellon::write_drills` / `excellon::read_excellon`,
`assembly::pick_and_place_csv` / `assembly::bom_csv` / `assembly::bom_lines`, and
`route::connector_points` / `route::connect`. Errors are `PcbError`, which says what was wrong and
where. All of these return `Result`; nothing in non-test code panics on input.

```text
let board = Board::from_json(&text)?;                 // bounded read, version checked, validated
let report = run_drc(&board, &CapabilityProfile::generic_2layer().rules())?;
for v in &report.violations { /* code, severity, locations (nm), object ids, measured, required */ }
let gerbers = gerber::write_all(&board, &GerberOptions::default())?;   // one file per layer
let drills = excellon::write_drills(&board, &DrillOptions::default())?;
```

## Units, angles and conventions

* All coordinates and sizes are `i64` nanometres (`units::Nm`). Coordinates are limited to
  +-2^30 nm (about +-1073 mm) and sizes to 2^30 nm, so exact predicates cannot overflow; anything
  beyond is an error. `x` grows to the right and `y` grows up, counter-clockwise is positive.
* Angles are **millidegrees** counter-clockwise (`90000` is a quarter turn). Multiples of 90 000 are
  exact; other angles use `f64` `sin`/`cos` and round each vertex to the nearest nm (half away from
  zero), so on a platform with a different libm the last nm of such a vertex could differ.
* A footprint on the back side is mirrored (`x -> -x`), then rotated, then translated. Its graphics
  and text move to the opposite layer.
* A size with an odd number of nm has its extra nanometre on the positive side.

## Verified reference cases

Everything below was run (`cargo test -p circuito-pcb`); nothing else is claimed.

* **Exact geometry.** Property tests (proptest) check symmetry, the triangle inequality, translation
  invariance, and agreement of the exact distance with brute-force floating-point sampling of the
  shapes, for points, segments with a width, polygons with holes and discs. Unit tests pin exact
  values at the nanometre, including diagonal distances where the answer depends on `2k^2 >= d^2`
  and half-nanometre boundaries of odd diameters.
* **DRC boundary tests.** One test per rule builds a tiny board whose gap is exactly the required
  value: the board **passes at `required`** and **fails at `required - 1 nm`** with exactly one
  violation of that rule. Rules covered: minimum track width, clearance for every object-kind pair
  (track-track, track-pad, track-via, pad-pad, pad-via, via-via, zone-copper), net-class
  overrides, shorts, annular ring (vias and pads, round, rectangular and slot), minimum drill,
  hole-to-hole, copper to board edge, objects outside the outline, keepouts, silkscreen over pads and
  silk width, solder mask bridge and sliver, courtyard overlap, unconnected nets (naming the pads and
  the net), dangling tracks, acute angles, unfilled zones, arcs, and the capability profiles.
  Clean boards are checked to produce no errors.
* **DRC speed.** A board of **20,000 tracks** is checked in about **0.15 s** in a debug build and
  about **0.035 s** in a release build (one thread, measured on the development machine); the planted single 1 nm violation among them is found and nothing
  else is reported. The test asserts a ceiling of 10 s. The candidate pairs come from a uniform grid
  I wrote (`drc::index`): the work is roughly linear in the number of objects for boards whose local
  density is bounded; a board where everything piles into one place degrades towards quadratic.
* **Gerber and Excellon.** The writers' files are read back with this crate's own readers and
  compared: flash positions, draw end points, arc end points and centres, region bounds (with clear
  holes), the aperture list (template, parameters, function), and file, aperture and object
  attributes, all **exactly**. A set of golden files (`tests/golden/`) pins every byte of the output
  for a fixed board; output is byte-deterministic (no date unless the caller supplies one). The
  readers are fed random and mutated text and never panic.
* **Hostile input.** Coordinates, sizes and counts beyond the limits are errors (tests with
  `i64::MAX`, negative and zero widths, absurd rules, oversized files, malformed JSON, unknown
  fields, a newer `format_version`).

**Not verified:** the output files have **not** been opened in an external CAM or Gerber viewer or
sent to a fabricator. They were checked against the specification text by reading and by this crate's
own readers, which share the author's understanding of the specification. Treat a first real order
accordingly, and run the files through the fabricator's CAM check.

## Gerber (Ucamco RS-274X / X2)

Written: `FSLAX46Y46` with `MOMM` (4 integer and 6 decimal digits, leading zeros omitted, absolute).
One unit of the coordinate integer is exactly 1 nm, so **no coordinate is ever rounded**; aperture
sizes are millimetres with six decimals, also exact. Apertures `C`, `R`, `O` and two macros
(`RECTROT`, a rectangle at any angle, and `RRECT`, a rounded rectangle at any angle); flashes, draws,
true `G75` arcs with `G02`/`G03` (a full circle when start equals end), regions `G36`/`G37`, clear
and dark polarity `LPC`/`LPD` (a zone's holes are clear regions), `M02`. X2 attributes:
`TF.GenerationSoftware,Nodalia,Circuito,<version>`, `TF.CreationDate` (only if you pass one),
`TF.SameCoordinates`, `TF.FileFunction` (`Copper,Ln,Top|Inr|Bot`, `Soldermask`, `Paste`, `Legend`,
`Profile,NP`), `TF.FilePolarity` (mask files are `Negative`: dark means opening), `TA.AperFunction`
(`Conductor`, `SMDPad,CuDef`, `ComponentPad`, `ViaPad`, `Profile`), `TO.N`, `TO.C`, `TO.P` (net,
reference, reference and pad; `N/C` for an unconnected pad). Attribute text is escaped (`\uXXXX`).
Objects are ordered regions, then draws, then flashes, each by aperture.

Layers written by `write_all`: every copper layer, mask and paste and silkscreen on both sides, and
the profile (skipped with a warning when there is no outline).

Not written, because nothing here needs them or they are out of scope: polygon apertures (`P`),
aperture blocks, step and repeat, `LM`/`LR`/`LS`, `TF.Part`, `TF.ProjectId`, `TF.MD5`, `TO.CN`,
`TO.CVal`, courtyard and fab layers, drill maps. The **reader** handles the dialect above (`FSLA`,
`MOMM`/`MOIN`, `G01`/`G02`/`G03` with `G75`, regions, polarity, attributes); it keeps macros as text
and does not evaluate them, and it refuses `G74`, trailing-zero formats, incremental coordinates and
the commands listed as unwritten, with an error that names the line.

Limits of the geometry written:

* A **custom pad** is written as a region. When mask expansion is positive the contour is also
  stroked with a round pen of twice the expansion (exact for a polygon grown by a disc); a negative
  expansion cannot shrink a custom polygon and leaves it unchanged.
* An **oval pad at an angle that is not a multiple of 90 degrees** is written as one stroke of a
  round aperture (so it is a draw, not a flash), tagged as a pad by its `AperFunction`.
* Rotated rectangles and rounded rectangles use the macros above, which describe the mathematically
  exact shape; the DRC uses the same shape with vertices rounded to the nanometre, so the two can
  differ by at most about 1 nm.
* Silkscreen is not clipped against pads or the mask: the DRC warns about silk over pads, the Gerber
  does not remove it.
* **Zones are not filled by this crate.** A zone is written from its `filled` regions; a zone with
  none is reported as a warning and left out. There is no thermal relief generator.
* Silkscreen text is drawn with a small stroke font of Circuito's own (capital letters, digits and
  common punctuation). Other characters are drawn as a box and counted in a warning.

## Excellon

`PTH` (through-hole pad drills and through vias), `NPTH`, and one file per blind, buried or micro
via layer span. Metric. Coordinates and tool diameters use an **explicit decimal point and six
decimals**, which is exact to the nanometre; because the point is explicit, leading- and
trailing-zero suppression do not apply and **no zero is ever suppressed** (the `TZ` in the header
declares the convention a reader would otherwise have to guess, and has nothing to act on). Round
holes are `X..Y..`; slots are the canned slot `X..Y..G85X..Y..`. Tools are sorted by diameter and
carry their X2 aperture function in a `; #@! TA.AperFunction` comment; holes are sorted and
duplicates removed. Not written: drill maps, route-mode outlines, backdrill, castellated holes. The
reader accepts the same dialect only (metric, decimal-point coordinates, `G85` slots) and refuses
inch files and integer coordinates with an error.

## Pick-and-place and BOM

CSV, RFC 4180 quoting, `\n` line ends. Pick-and-place columns `Ref,Value,Package,PosX,PosY,Rot,Side`
(millimetres with six decimals, rotation in degrees counter-clockwise with three decimals and no
mirror correction for the back side, an optional origin offset, SMD parts only by default). The BOM
groups by value and package: `Qty,Value,Package,References`, references in natural order (`R2`
before `R10`). Text that a spreadsheet would run as a formula (a leading `=`, `+`, `-`, `@`) gets a
leading apostrophe. A missing or repeated reference designator is an error naming the footprint.
IPC-2581 is not implemented.

## The connector helper

`route::connector_points` returns the one or two segments joining two points at angles of 0, 45 or
90 degrees (four shapes), exactly, in integers. `route::connect` builds the tracks and runs the DRC
on a copy of the board, returning the violations that involve the new tracks (clearance, shorts,
width, edge, keepout). It does not change the board and it is not an autorouter: there is no search,
no obstacle avoidance and no multi-layer routing.

## Limits and honest gaps

* No zone fill, no thermal relief, no teardrops; differential-pair rules (gap, width) are stored on
  the net class but **not enforced** by the DRC; no length matching or impedance checks.
* The DRC approximates arcs by polylines within `Rules::arc_tolerance` (default 1 um sagitta); a
  clearance to an arc can therefore be off by up to that tolerance plus 1 nm. The Gerber keeps
  true arcs.
* An arc's start and end radii may differ by up to 2 nm (centres are rounded to the nanometre).
* Caps (an error beyond them): 1,000,000 objects of each kind, 20,000 pads per footprint, 50,000
  vertices per polygon ring, 10,000 holes per polygon, 512 bytes per name, 20,000 segments per
  approximated arc, 256 characters per text, 128 MiB per `.circuito-pcb` file, 256 MiB per Gerber
  file, 128 MiB per Excellon file, 100,000 apertures per file.
* Solder-mask checks consider pad and via openings; they do not consider slivers against the board
  edge or silk-to-mask rules. The courtyard check is an interior-overlap test of the placed
  polygons.
* Capability profiles (`generic-2layer-6-6`, `generic-4layer-5-5`) are Circuito's own conservative
  values, not copied from any manufacturer. They are a starting point, not a promise that a given
  fabricator accepts them.
* No 3D, no STEP, no IPC-2581, no ODB++, no import of other tools' board formats.

## Layout

| Module | Contents |
|---|---|
| `units`, `error` | nanometre helpers, exact decimal parsing and formatting, `PcbError` |
| `geom` | points, exact predicates and wide integers, regions, arcs, clearance shapes |
| `layer`, `stackup`, `board`, `io` | the model, validation and the `.circuito-pcb` file |
| `transform`, `artwork`, `font` | placement, pad shapes, graphics and the stroke font |
| `drc` | rules, profiles, spatial grid, checks, violations |
| `gerber`, `excellon` | writers and readers |
| `assembly`, `route` | CSV outputs and the connector helper |

Tests are in `tests/` (model and file round trips, geometry properties, DRC boundaries, DRC at scale
and on hostile input, Gerber, Excellon, golden files, assembly and routing, reader fuzzing) next to
the unit tests in `src/`. To regenerate the golden files after a deliberate change to the output:
`CIRCUITO_BLESS=1 cargo test -p circuito-pcb --test gerber_golden`, then review the diff.
