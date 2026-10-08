# AGENTS.md

Guidance for coding agents (and humans) working on afterslides. Keep this file
current: when a rule here turns out wrong or a new gotcha costs time, fix it
in the same change.

## What this project is

A library for **filling PowerPoint templates with data**: find shapes, tables
and native charts in a `.pptx`, replace text, table rows and chart series,
delete shapes or slides that have no data, save. It replaces a commercial PPTX
library in report-generation services.

Out of scope (for now): rendering to PDF/images, creating decks from scratch,
legacy `.ppt`, animations. Use LibreOffice for rendering.

## Layout

```
crates/afterslides/      Rust core (published crate). No Python knowledge.
  src/xml.rs             Mutable XML tree; keeps qualified names as written.
  src/opc.rs             Zip container, content types, relationships, GC.
  src/presentation.rs    Presentation, SlideId/ShapeRef handles, save/open.
  src/shape.rs           Shape tree walking, shape text, shape deletion.
  src/text.rs            Text bodies; cross-run placeholder replacement.
  src/table.rs           Table read/fill/resize.
  src/chart.rs           Chart caches, series cloning, embedded workbook.
  src/slide.rs           Delete/duplicate/move slides and every reference.
  tests/template.rs      Integration tests on tests/fixtures/template.pptx.
crates/afterslides-py/   PyO3 bindings (`afterslides._native`), handle-based.
python/afterslides/      Public Python API (`_api.py`), errors, `_native.pyi`.
tests/                   pytest suite; python-pptx/openpyxl/LibreOffice as oracles.
scripts/make_fixtures.py Regenerates tests/fixtures/template.pptx.
docs/                    Architecture and migration notes.
```

## Commands

```sh
uv venv && uv pip install maturin pytest python-pptx openpyxl ruff mypy
export PYO3_PYTHON=$PWD/.venv/bin/python     # needed for cargo on the -py crate

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p afterslides                     # core only; the cdylib has no test harness
uv run --no-sync maturin develop              # build + install the extension (debug)
uv run --no-sync pytest -m "not slow"         # quick
uv run --no-sync pytest                       # incl. memory soak and LibreOffice
uvx ruff check && uvx ruff format --check && uvx mypy --strict python/afterslides
uv run python scripts/make_fixtures.py        # after changing the fixture script
```

CI (`.github/workflows/ci.yml`) runs all of the above on Linux, macOS and
Windows plus an MSRV check (Rust 1.88). Keep it green; don't merge red.

## Hard rules

- **Round-trip fidelity.** Parts we don't modify must be written back byte for
  byte (`untouched_round_trip_is_lossless`). Parts we modify must keep every
  element and attribute we don't understand, and **namespace prefixes exactly
  as written**: `mc:Ignorable="p14 a16"` refers to prefixes by name, and
  re-prefixing breaks PowerPoint. Create new elements with
  `Element::new_like(sibling_or_parent, ...)` to borrow the right prefix.
- **Only mark a part dirty when it really changes.** `Package::xml_mut` marks
  the part dirty; check with `xml()` first when a change is conditional (see
  `replace_text_on_slide`).
- **No memory leaks.** `unsafe` is forbidden in the workspace, `mem::forget`
  is denied, no `Box::leak`, no `Rc`/`Arc` cycles. Python wrappers only point
  "upwards" (Shape -> Slide -> Presentation -> native), never back. The soak
  test in `tests/test_memory.py` must stay green.
- **No competitor names.** Never write the name of the commercial library we
  replace in code, docs, commits or package metadata. Say "a commercial PPTX
  library".
- **Commits** are small, imperative subject (≤ 72 chars), body explains why.
  No tool or assistant attribution lines.
- **License headers**: none per file; the project is LGPL-3.0-or-later
  (`COPYING`, `COPYING.LESSER`, `LICENSE.md`).

## OOXML gotchas (learned the hard way)

- **Deleting a slide** must remove: the `p:sldId`, its relationship in
  `presentation.xml.rels`, the part and its rels, its `[Content_Types].xml`
  override, its `p14:sldId` in `p14:sectionLst`, `p:custShow` entries, and
  hyperlinks on other slides that target it. A stale section entry triggers
  PowerPoint's repair prompt. Media can be shared between slides, so parts
  are dropped by reachability GC on save, never directly.
- **Sections must stay contiguous** and in slide order; after inserting or
  moving a slide, `place_in_sections` puts it into its neighbour's section.
- **Duplicated slides** need their own chart parts (and embedded workbooks),
  otherwise filling one chart changes both. Notes are copied and their back
  reference re-pointed. Comments are dropped.
- **Chart series** must have unique `c:idx`/`c:order`; copies also need fresh
  `c16:uniqueId` values. Remove `c:dPt`/`c:dLbl` whose index is past the new
  point count.
- **Chart caches vs workbook**: PowerPoint draws from `c:strCache`/`c:numCache`;
  the embedded xlsx is only used for "Edit Data". Both must agree, and the
  `c:f` formulas must point at the cells we wrote (`Sheet1!$B$2:$B$5`).
- **Table rows** carry `a16:rowId` in `a:extLst`; copied rows get new ids.
  Keep `p:graphicFrame/p:xfrm/a:ext/@cy` equal to the sum of row heights.
- **Text conventions** follow python-pptx: `\n` separates paragraphs, `\v` is
  a line break (`a:br`).
- `mc:AlternateContent` can contain the same shape twice (Choice and
  Fallback). Edits apply to both (`with_shape_mut`); deletion removes the
  whole `mc:AlternateContent`.
- python-pptx writes `'` quotes in the XML declaration; our writer uses `"`.
  That's fine, but don't compare XML of modified parts byte-wise in tests.

## Testing approach

- Rust tests check behaviour through the public core API.
- Python tests verify output with **independent readers**: python-pptx for
  slides/charts/tables, openpyxl for embedded workbooks, LibreOffice for
  "does another office suite open it".
- Add a case to `scripts/make_fixtures.py` when a bug needs a template
  feature the fixture lacks; regenerate and commit the `.pptx`.
- Real-world templates from users are the best test material. Anonymize them
  before adding to `tests/fixtures/`.
