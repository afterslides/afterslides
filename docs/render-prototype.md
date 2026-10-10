# Rendering prototype: findings

**Question:** can afterslides render slides to PDF and PNG with a pure-Rust
stack, well enough for report decks, and at what cost?

**Answer: yes, go.** The prototype draws shapes, pictures, text, tables and
the common chart types, renders every deck of the real-world corpus without
a single failure, and adds about 2 MB to the wheel. Fidelity is
"recognisably the same slide" today; the gaps are known and listed below.

## What was built

`crates/afterslides/src/render/` (feature `render`, ~5,000 lines):

```
OOXML (slide, layout, master, theme)
  -> scene/shapes/text/table/chart: resolve inheritance, geometry, layout
  -> display list (paths, strokes, images, glyph runs, clips)
  -> pdf.rs (krilla)      -> PDF with embedded, subsetted fonts and real text
  -> raster.rs (tiny-skia) -> PNG
```

| Area | Implemented |
| --- | --- |
| Inheritance | slide -> layout -> master placeholders (position, text, body properties), `showMasterSp`, colour map overrides |
| Colours | theme colours, `clrMap`, `lumMod/lumOff/tint/shade/alpha/sat/hue`, `phClr` in theme styles |
| Geometry | all ~187 preset shapes via an evaluator over the ECMA-376 definitions, custom geometry, arcs, rotation, flips, groups |
| Fills and lines | solid, linear/radial gradient, picture (with crop), pattern (as solid), theme style references, dashes, caps, joins |
| Text | full style chain (default text style, master title/body/other styles, font reference, list styles, paragraph, run), parley layout, alignment, line spacing, first-line indent, bullets and numbering, breaks, slide number fields, hyperlink colour, underline/strike, stored autofit, font substitution (only for fonts that are not installed) |
| Tables | grid, row growth, merges, fills, borders, margins, anchoring, table styles; built-in default style approximated |
| Charts | column/bar (clustered, stacked, percent), line with markers, area, pie, doughnut; title (incl. automatic), legend, value axis scaling (5% headroom, nice major unit: matches LibreOffice's output for the fixture; PowerPoint's exact rule unverified), number formats, gridlines, category labels |
| Output | `Presentation.render_pdf()`, `Slide.render_png(scale)`; hidden slides skipped unless asked |

## Measurements

### Fidelity against LibreOffice

`scripts/render_compare.py` renders the fixture with LibreOffice (PDF, then
`pdftoppm`) and with afterslides, at 72 dpi, and reports the RMSE per slide.
Both sides use the same fonts (Carlito for Calibri).

| Slide | Content | Blank page | Prototype |
| --- | --- | ---: | ---: |
| 0 | title, subtitle with mixed formatting | 6.6% | **4.9%** |
| 1 | column chart, text box | 18.9% | **11.7%** |
| 2 | table | 13.5% | **7.5%** |
| 3 | line chart, pie chart, text | 13.7% | **8.5%** |
| 4 | scatter, bubble chart (not drawn yet) | 8.7% | 8.6% |
| 5 | group, picture | 9.9% | **2.7%** |
| 6 | hyperlinked text | 2.4% | 2.4% |
| **mean** | | **10.5%** | **6.6%** |

LibreOffice is not PowerPoint: several remaining differences are LibreOffice
deviating from PowerPoint (it drops the bold header row of the default table
style, draws chart areas grey, uses its own chart text sizes). The numbers
measure "roughly right", not fidelity to PowerPoint.

### Robustness

- All **143** readable decks of the corpus (python-pptx and Apache POI test
  files) render to PDF and PNG: **0 panics, 0 errors**, in release and in
  debug builds (debug catches integer overflow). The 12 other files are
  broken on purpose and rejected when opening.
- One overflow panic was found and fixed while building the table renderer;
  the corpus run is part of CI now.

### Speed

- 697 corpus slides to PDF *and* PNG in 10.1 s: **~14 ms per slide** (release).
- The fixture (7 slides) renders to PDF in **~3 ms** once the system font
  list is loaded; loading it takes ~25 ms and happens once per process.

### Package size and dependencies

| Wheel (Linux x86-64) | Size |
| --- | ---: |
| without rendering | 1.4 MB |
| with rendering | 3.5 MB |

- No native graphics libraries: the extension links only libc, libm and
  libgcc. fontique originally linked fontconfig (plus freetype, harfbuzz,
  glib), which rules out manylinux wheels; with `fontconfig-dlopen` it is
  loaded at runtime when present.
- **manylinux verified:** built in the official `ghcr.io/pyo3/maturin`
  container, the wheel (rendering included) is tagged
  `manylinux_2_17_x86_64.manylinux2014_x86_64`, is 3.5 MB, installs into a
  fresh environment and renders. (A local build on a very recent glibc gets
  a plain `linux` tag; release wheels are built in the container.)
- Clean release build in that container: 1 min 21 s with rendering.
  Incremental local builds: ~27 s with rendering vs ~22 s without.

### Platforms

CI builds rendering on Linux, macOS and Windows with Python 3.9 and 3.13.
The PNG and robustness tests run on all three; the tests that check PDF
text need poppler's `pdftotext` and run on Linux only. The render unit tests
(geometry, colours, number formats, axis scales, font substitution) run in
the Rust CI job. Rendering needs Rust 1.92 (krilla); the core crate without
the feature stays at 1.88.

### Memory

The soak test (200 open/fill/save/render cycles) shows flat memory. The
shared font list is a bounded, process-wide cache.

### PDF output

Fonts are embedded as subsets with Unicode maps, so text is searchable and
copyable (`pdftotext` returns the slide text). The fixture's PDF is 24 KB.
PDF and PNG backends agree within 1–4% RMSE (anti-aliasing and hinting).

## Not done yet (in order of impact for report decks)

1. **Chart details:** data labels, scatter and bubble charts, secondary axes,
   axis titles, manual chart layouts, chart text from `c:txPr` per element.
2. **Effects:** shadows, glow, soft edges, reflections. tiny-skia has no
   blur; this is the one feature where Skia would be clearly easier.
3. **Text:** vertical and rotated text (`bodyPr vert/rot`), East Asian and
   complex-script fonts per script (`a:ea`, `a:cs`), CJK line-breaking data
   (parley's `complex-scripts` feature), character spacing, tab stops.
4. **Lines:** arrow heads (`headEnd`/`tailEnd`), compound lines.
5. **Images:** EMF/WMF (common in corporate decks), SVG originals (we draw
   the PNG fallback), picture effects and recolouring.
6. **SmartArt:** drawn from the cached drawing part.
7. **Fonts:** a Python API for font directories and substitutions (the Rust
   API has it; font directories are read once, when fonts are first loaded).

## Recommendation

- **Keep the pure-Rust stack.** Packaging is trivial: there is no C or C++
  code that we compile or ship, our own crates forbid `unsafe`, and system
  font discovery goes through the OS (fontconfig loaded at runtime,
  CoreText, DirectWrite). Dependencies such as tiny-skia and the JPEG
  decoder do use `unsafe` internally, as all graphics libraries do. The
  quality ceiling is set by our OOXML interpretation, not by the drawing
  library. Revisit Skia only if shadows, glow and soft edges become a
  must-have.
- **Get PowerPoint reference PDFs.** With PowerPoint-exported PDFs of the
  fixture and a few corpus decks, the comparison measures what matters.
  One-off access to PowerPoint is enough.
- **Next milestone for rendering:** chart details (1) and arrow heads (4),
  then a font configuration API (7). That covers what report decks contain.
- **Ship it as part of the main wheel** (+2 MB) rather than as a separate
  package; it can be built without rendering for those who don't want it.
