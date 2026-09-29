# Reconstruction heuristics

Everything is a threshold on measured geometry or typography. Thresholds are
in ems of the local font size unless noted. Code: `crates/reconstruction`.

## 0. Glyph preparation (`glyphs.rs`)
* Drop PDFium-generated characters (keep generated spaces only as weak word
  hints), invisible text, and control characters.
* Rotated glyphs (> 1°) are set aside as raster-fallback regions.
* Real space glyphs become a `space_before` flag; ligatures (ﬁ, ﬂ, …) are
  expanded with their advance split evenly.
* Font sizes are rounded to 0.05 pt so run styles merge.

## 1. Glyph → word → line fragment (`lines.rs`)
* Baseline clustering: same line if baselines differ by ≤ 0.25 em.
* Overprinted duplicates (same char within 0.4 pt) are removed and mark the
  glyph fake-bold; fill+stroke text is fake-bold too.
* Word break: advance gap > 0.15 em, or a real space glyph.
* Fragment break: gap > 1.2 em, or a vertical ruling line between glyphs.
  Two fragments on one baseline are later classified as a tab row, columns or
  a table.

## 1b. Graphics (`graphics.rs`)
* Axis-aligned thin paths → horizontal/vertical rules; stroked rectangles
  contribute four rules.
* A thin rule within 0.3 em below a baseline, inside the line's extent →
  underline (consumed).
* Small filled squarish paths left of a line → vector bullets, converted to a
  "•" marker sized/shifted to match the drawn disc.

## 2. Zones (`zones.rs`)
Lines are grouped into bands (transitive vertical overlap). A top-to-bottom
sweep opens a candidate at a band with internal gaps ≥ 0.6 em and keeps it
open while its gutters stay free of lines (narrowing them). A band that
crosses only some gutters ends a table but only narrows a column candidate.
On close:
1. last column right-aligned (x1 spread ≤ 2 pt) and ≥ 80% of its lines share
   a baseline with another column → **rejected** (resume dates: tab rows);
2. ≥ 80% of lines row-aligned across columns and ≥ 2 rows → **table**;
3. every column has ≥ 2 lines (independent flows) → **columns**, recursing
   into each column (depth ≤ 3);
4. otherwise rejected.
Rejection backtracks: only the opening band becomes flow and the sweep
resumes after it.

## 3. Paragraphs (`paragraphs.rs`)
* Fragments sharing a baseline inside a flow merge into one line with tab
  stops: right tab if the last part ends at the flow's right edge (±3 pt),
  centre tab if centred, else left tab.
* Lines overlapping an earlier line (text over text) become absolute frames.
* List markers: first word is a bullet glyph (incl. Symbol/Wingdings private
  use), or an ordinal `1.`, `2)`, `(3)`, `a.`, `iv.` (≤ 2 digits; letters and
  numerals need a ≥ 0.4 em gap). A bullet glued to its text is split off.
* A line continues the previous paragraph only if all hold:
  no marker, no tabs; same majority size (±5%), boldness and italics; pitch ≤
  1.2× the established pitch (≤ 1.6 em for the second line) and ≥ 0.6 em;
  left edge within 1.5 pt of the continuation edge (text after a marker for
  list items; a first-line indent up to 4 em is allowed); and **wrap
  evidence**: the next line's first word would not have fit at the end of the
  previous line. Centred lines continue centred paragraphs by width.
* Alignment: centre if all lines are centred (±2 pt) away from the left edge;
  justify if ≥ 3 lines, equal lefts, all but the last ending at the right
  edge; right for a single line ending at the edge and starting past 30% of
  the width; else left.
* Letter spacing: median intra-word gap when > 0.02 em.
* Line pitch: median baseline delta; single-line paragraphs use the pitch to
  the next same-size line, or the flow's measured pitch for that size, or
  1.2 em.
* Headings (role only): ≤ 2 lines, ≤ 90 chars, no list/tabs, and size among
  the page's heading sizes (short lines ≥ 1.15× body size) → level by size
  rank; or bold all-caps at body size when body text is not bold.
* `space_before` uses the shared box model (see ARCHITECTURE.md).

## 4. Tables and columns (`lib.rs`)
* Ruling grid = rules connected (touching) to a vertical rule near the table
  text, so wide last columns keep their far border and an unconnected divider
  above the table is not captured. With n+1 vertical rules they define the
  column boundaries; otherwise midpoints between column text. Horizontal
  rules define row boundaries when there are rows+1 of them.
* Columns become `Columns { x, width, blocks }` relative to the parent flow.

## 5. Page level
* Horizontal rules and images are assigned to the nearest flow zone that
  horizontally contains them; images with text beside them become
  decorations. Everything unconsumed becomes a page decoration.
* Margins: left/right from the text extent, top from the first block,
  bottom from the last block (clamped to the top margin).
