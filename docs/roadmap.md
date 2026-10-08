# Roadmap

Priorities follow the template-filling use case: real corporate templates
must come out of afterslides without PowerPoint asking to repair them.
Open an issue if you need something that is further down.

## M2: no corrupt output (in progress)

Checks first, so CI sees what PowerPoint would complain about:

- Invariant checks on every test output: every `r:*` id resolves, every
  relationship target exists, unique `c:idx`/`c:order` per chart (including
  filtered series), unique shape ids per slide, no row/column spans past the
  table, a content type for every part.
- Validate test outputs with the Open XML SDK in CI.

Fixes for known ways to produce broken files:

- Relationship targets that are percent-encoded or differ in case from the
  part name.
- Duplicate shape ids on a slide (PowerPoint tolerates them; we must not edit
  or delete the wrong shape).
- Slide duplication with modern comments (`p188:commentRel`).
- New chart series colliding with filtered series (`c15:filteredSeries`).
- `p:txBody` inserted after `p:extLst` when setting text on an empty shape.
- Non-finite numbers in chart data.
- Tables with vertically merged cells when growing or shrinking.
- Combo charts: refuse ambiguous series changes until plots can be targeted.

## M3: real-world fixture corpus

Anonymized decks saved by PowerPoint (and other suites) covering
`mc:AlternateContent`, chart style/colour parts, `themeOverride`, data point
overrides, combo charts and secondary axes, date axes, multi-level
categories, merged cells, comments, custom shows, SmartArt, OLE, media and
animations.

## M4: chart completeness

- Choose the plot (and template series) a new series is copied from.
- Filtered series, multi-level categories, data labels from cell ranges.
- Edit the embedded workbook in place, keeping its formatting and formulas.

## M5: API freeze for 1.0

- Settle shape-handle semantics and text conventions (`\n`/`\v` everywhere,
  including `replace_text`).
- `#[non_exhaustive]` on public Rust data types; keep the XML and package
  layers internal.
- Placeholder text in notes, layouts and chart titles; hide slides; replace
  pictures; find placeholders by type/index.

## M6: releases

- Wheels and sdist on tags, PyPI and crates.io publishing.
- Faster saves for large templates (copy untouched zip entries without
  recompressing), benchmarks on large decks.
- `cargo deny`/audit in CI, fuzzing the XML and package readers.

## Not planned

- Legacy binary `.ppt`.
- Building decks from scratch (use python-pptx for that).
- Rendering to PDF/images (convert with LibreOffice).
