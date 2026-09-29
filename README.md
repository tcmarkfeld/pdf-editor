# Reflow

A native desktop editor that opens a PDF and turns it into an editable,
reflowing document — paragraphs, lists, tab-aligned rows, columns, tables —
using deterministic geometry and typography heuristics (no AI/OCR/ML).
Optimised for resumes and similar business documents. Rust, PDFium, Parley,
egui on wgpu, krilla.

## Setup

```bash
scripts/fetch-pdfium.sh
```

```bash
cargo run --release -- fixtures/pdf/resume-single.pdf
```

## Building the Mac app

```bash
scripts/package-macos.sh --install
```

This builds and installs `/Applications/Reflow.app`, registered so Finder's
*Open With* lists it for PDFs .
`--dmg` additionally produces `dist/Reflow.dmg` for copying elsewhere. The app is self-contained: PDFium
is bundled in `Contents/Frameworks`. It is signed ad-hoc, which is enough
for your own Mac; to give it to others, sign with a Developer ID certificate
and notarize (`codesign --options runtime --sign "Developer ID Application: …"`
then `xcrun notarytool submit --wait` and `xcrun stapler staple`).

## Using it

Click into any text and type. Enter splits paragraphs (and continues lists),
Backspace merges, Tab types a tab (or moves between table cells / nests list items), arrows/⌥arrows/⌘arrows navigate, ⇧ extends
selections, ⌘Z/⇧⌘Z undo/redo, ⌘C/⌘X/⌘V clipboard (styles preserved within
the app), ⌘B/⌘I/⌘U toggle styles, ⌘S saves the edited PDF in place (⇧⌘S saves
it somewhere else), ⌘+/⌘−/pinch zoom. *View → Reconstruction debugger* shows the original
render, a difference overlay and structure boxes.

### Formatting toolbar

Undo/redo, paragraph style (Normal, Heading 1–3), font and size, bold /
italic / underline / strikethrough, text colour, links, alignment, line
spacing, bulleted and numbered lists (nesting with Tab / Shift+Tab or the
indent buttons), insert table (grid picker), horizontal line, and clear
formatting. With the caret in a table a second bar adds rows/columns above,
below, left or right, deletes rows/columns/the table, toggles borders and
their colour, shades cells or rows, and distributes columns evenly. Tab moves
to the next cell (and adds a row from the last one).

Shortcuts: ⇧⌘7 / ⇧⌘8 numbered / bulleted list, ⌥⌘0–3 Normal / headings,
⌘[ / ⌘] indent, ⇧⌘L/E/R/J alignment, ⇧⌘X strikethrough, ⌘K link, ⌘\ clear
formatting. With nothing selected, formatting applies to what you type next.

## CLI

```bash
cargo run -- outline fixtures/pdf/resume-two-column.pdf
```

```bash
cargo run -- compare fixtures/pdf/resume-single.pdf /tmp/cmp
```

`convert <in.pdf> <out.reflow>` and `export <in> <out.pdf>` are also
available.

## Tests

```bash
cargo test --workspace
```

Synthetic fixtures with exactly known geometry live in
`crates/reconstruction/tests`; realistic fixtures (`fixtures/pdf`) are
generated from `fixtures/html` with WebKit via `scripts/html2pdf.swift`.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[docs/RECONSTRUCTION.md](docs/RECONSTRUCTION.md).
