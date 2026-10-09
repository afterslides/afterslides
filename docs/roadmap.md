# Roadmap

Priorities follow the template-filling use case: real corporate templates
must come out of afterslides without PowerPoint asking to repair them.
Open an issue if you need something that is further down.

## Done

- **M2, no corrupt output**: invariant checks and Open XML SDK validation on
  every test output; fixes for percent-encoded targets, duplicate shape ids,
  comments on copied slides, filtered series, element order, table merges,
  combo charts, animations of deleted shapes.
- **M3, real-world corpus**: ~150 decks from python-pptx and Apache POI,
  every edit applied to each, results compared with their input
  (`docs/testing.md`).
- **M4, chart completeness**: plot targeting in combo charts, multi-level
  categories, number format overrides, date categories, filtered series.
- **M5, API for 1.0**: private XML/package layers, `#[non_exhaustive]`
  data types, newline conventions in `replace_text`, notes and chart text,
  hidden slides, placeholders by type/index, picture replacement, live
  shape handles.

## M6: releases

- Wheels and sdist on tags, PyPI and crates.io publishing.
- Faster saves for large templates (copy untouched zip entries without
  recompressing), benchmarks on large decks.
- `cargo deny`/audit in CI, fuzzing the XML and package readers.

## Later

- Edit the embedded workbook in place instead of regenerating it, keeping
  workbook styles, Excel tables and defined names. Deferred: the workbook
  is only visible behind *Edit Data*, and keeping tables and names
  consistent is a lot of risk for little visible benefit.
- Data labels from cell ranges (`c15:datalabelsRange`), custom error bars.
- Adding a plot to, or removing one from, a combo chart.

## Not planned

- Legacy binary `.ppt`.
- Building decks from scratch (use python-pptx for that).
- Rendering to PDF/images (convert with LibreOffice).
