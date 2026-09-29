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

## Using it

Click into any text and type. Enter splits paragraphs (and continues lists),
Backspace merges, Tab types a tab, arrows/⌥arrows/⌘arrows navigate, ⇧ extends
selections, ⌘Z/⇧⌘Z undo/redo, ⌘C/⌘X/⌘V clipboard (styles preserved within
the app), ⌘B/⌘I/⌘U toggle styles, ⌘S saves a `.reflow` document, ⌘E exports
a PDF, ⌘+/⌘−/pinch zoom. *View → Reconstruction debugger* shows the original
render, a difference overlay and structure boxes.

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
