# Architecture

Revise opens a PDF, reconstructs it into an editable document model with
deterministic geometry/typography heuristics, and from then on treats the
model — not the PDF — as the source of truth. Layout, editing, rendering and
export all read the model.

```
            ┌────────────┐   SourcePage    ┌────────────────┐   Section    ┌──────────┐
 file.pdf ─►│ pdf_source │───────────────►│ reconstruction │─────────────►│ document │◄── .revise (JSON)
            └────────────┘ (glyphs, fonts, └────────────────┘              └────┬─────┘
             PDFium          paths, images,                                     │ edits (editor)
             reference       links)                                             ▼
             renders                                  ┌───────┐  fonts   ┌──────────┐
                                                      │ fonts │◄────────►│  layout  │ Parley shaping,
                                                      └───────┘          └────┬─────┘ flow, pagination
                                                                              │ DocLayout (positioned items)
                                                    ┌─────────────────────────┼───────────────┐
                                                    ▼                         ▼               ▼
                                              render::gpu               render::raster    export
                                              (glyph atlas → egui/wgpu) (tiny-skia CPU)    (krilla PDF)
```

## Crates

| crate | responsibility | key dependencies |
|---|---|---|
| `document` | Model types, run-level text ops, tree addressing, JSON persistence, outline dump | serde |
| `pdf_source` | PDFium wrapper → plain `SourcePage` data; reference rendering; font-name parsing | pdfium-render 0.9 (PDFium 7881) |
| `reconstruction` | Deterministic `SourcePage → Section` pipeline + debug analysis + confidence scores | — |
| `fonts` | Source font → installed font resolution, substitution log | parley/fontique |
| `layout` | Shaping, tab segments, lists, flow, columns, tables, frames, pagination, hit testing | parley 0.11 |
| `editor` | Selections, commands, navigation, clipboard, snapshot undo/redo | unicode-segmentation |
| `render` | Glyph cache (swash), GPU atlas meshes, CPU rasterizer + visual diff | swash, tiny-skia, etagere, epaint |
| `export` | Laid-out pages → PDF with subset fonts, vector text, images, links | krilla 0.8 |
| `app` | eframe/wgpu window, background worker, canvas, debug panel, CLI | eframe 0.36 (wgpu), rfd |

Layering is enforced by crate dependencies: `reconstruction` never sees
PDFium types or fonts; `editor` never renders; `layout` never reads the PDF.

## Decisions (and where the spec was changed)

**PDFium over MuPDF.** PDFium is BSD/Apache licensed (MuPDF is AGPL), exposes
per-character loose/tight boxes, origins, effective font size, weight and
flags, and has a mature Rust wrapper. It is loaded dynamically from
`vendor/pdfium` (see `scripts/fetch-pdfium.sh`), the executable's directory,
`../Frameworks` (app bundle) or `PDFIUM_DYNAMIC_LIB_PATH`. PDFium is not
reentrant, so each open document lives on one worker thread.

**Coordinates are points, top-left origin** (not 0..1 normalized). All
reconstruction thresholds are expressed in ems of the local font size, which
needs absolute units.

**Lists and headings are paragraph properties, not containers** (as in Word
and Google Docs). Enter/Backspace/merge work uniformly; list numbering is
computed at layout time from consecutive paragraphs sharing a list id.

**Resume rows are tab stops, not a row container.** `FirmPilot ⟶ 2024 –
Present` becomes one paragraph `"FirmPilot\t2024 – Present"` with a right tab
stop. Editing the left side reflows naturally and the date stays right
aligned; the caret moves through the row like any text. Containers are used
only where flows are genuinely independent: `Columns`, `Table` cells, and
absolutely positioned `Frame`s (the low-confidence escape hatch).

**Exact vertical fidelity via a shared box model.** Parley breaks lines and
positions glyphs horizontally; the `layout` crate owns vertical placement.
A line of pitch `P` whose largest font is `S` puts its baseline `0.2·S` above
the line box bottom (`document::DESCENT_RATIO`). Reconstruction measures
`space_before` with the same rule, so every imported baseline lands exactly
where the PDF had it regardless of the fallback font's ascent/descent.

**One section per source page.** Sections never merge or split during
editing. If content outgrows its page, layout adds continuation pages (same
size and margins); shrinking content does not pull the next page up. This
preserves original page design and gives stable section indices (used for
background reconstruction and layout caching).

**Snapshot undo with structural sharing.** `Document.sections` is
`Vec<Arc<Section>>`; edits `make_mut` only the touched section. A snapshot is
a vector of Arcs, typing coalesces into one step. The same Arc identity keys
the per-section layout cache, so an edit re-lays out one page.

**GPU rendering through egui's wgpu renderer.** Glyphs are rasterized with
swash at the exact device-pixel size for the zoom (¼-pixel horizontal
positioning, whole-pixel baselines), packed into a 2048² atlas with etagere
and submitted as textured meshes. Vector decorations (backgrounds, icons) are
rasterized per zoom level with tiny-skia into a page layer texture — a custom
wgpu pipeline would only be worth it if profiling shows the mesh path is a
bottleneck.

**Background import with immediate display.** On open, the worker reports
page sizes, the canvas shows PDFium bitmaps of pending pages right away, and
each page is swapped for its reconstruction as soon as it is ready (≈1 ms per
resume page). Installing a reconstruction is not an edit: it is patched into
undo history too.

**Export re-uses the shaped fonts.** Parley's resolved font blobs and glyph
ids go straight to krilla, which subsets and embeds them with ToUnicode maps,
so the exported PDF has selectable text and identical glyph positions to the
screen (advances are derived from final positions, preserving justification
and letter spacing).

## Font handling

`pdf_source::fontname` parses BaseFont names (`ABCDEF+TimesNewRomanPS-BoldMT`
→ family "Times New Roman", weight 700). `fonts::FontSystem` resolves each
request through a fixed candidate list: installed family → metric-compatible
alias (Calibri→Carlito, Arial→Liberation Sans/Arimo, …) → generic class
(serif/sans/mono from PDF flags and name) → system fallback. Parley then
does per-cluster fallback for characters the family lacks. Every non-exact
resolution is listed in the debug panel and by `revise compare`.

## Confidence and fallbacks

Each paragraph gets a `ReconstructionScore { geometry, typography, spacing }`
(fractions of measured evidence consistent with the chosen structure), shown
by the debug tooling. Column and table zones are only accepted when their
classification rules hold (see `RECONSTRUCTION.md`); otherwise content stays
ordinary flow. Content that cannot participate in flow is preserved rather
than corrupted:

* text drawn over other text → absolutely positioned `Frame` (still editable);
* rotated text → raster crop of the source page as a decoration;
* unrecognised vector graphics and images beside text → page decorations.

## Debug tooling

* GUI: *View → Reconstruction debugger* — original render overlay with
  opacity, difference overlay (red: original only, blue: reconstruction
  only, with per-page % scores), and boxes for glyphs, words, lines, blocks
  and zones; font substitution list; reconstruction notes.
* CLI: `revise outline`, `revise compare <pdf> <dir>` (original /
  reconstructed / diff PNGs + scores), `revise convert`, `revise export`.
* `REVISE_SCREENSHOT=out.png REVISE_EXIT=1 [REVISE_SCRIPT='click:0:100:200;type:x;key:Enter']`
  captures the window after scripted input (`scripts/shot.sh`).

## Known limitations

* Embedded PDF fonts are not reused; text is re-shaped with installed
  equivalents (subset fonts usually lack glyphs for new text and often have
  unusable cmaps). With a missing font, line breaks after import can differ
  from the source when the substitute is wider. Planned: reuse embedded
  TrueType programs with valid Unicode cmaps, and per-font letter-spacing
  calibration from source advances.
* Line-end hyphens are kept (no dictionary to tell soft from hard hyphens).
* Table cells are single-row-band; multi-line cells without ruling lines
  become separate rows. Nested tables, merged cells, and rotated pages are
  not reconstructed specially.
* Single-word lines in a very narrow column are indistinguishable from
  wrapped text and merge into one paragraph unless typography differs.
* RTL/vertical scripts are shaped correctly by Parley but reconstruction
  assumes left-to-right horizontal text.
* Paragraphs that span two source pages remain two paragraphs.
