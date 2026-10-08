# Roadmap

Priorities follow the template-filling use case. Items move up when someone
needs them; open an issue if that's you.

## Next

- Faster saves for large templates: copy untouched zip entries without
  decompressing and recompressing them.
- Combo-chart control: choose which plot new series go into.
- Multi-level categories (`c:multiLvlStrRef`) for read and write.
- Replace pictures (logos, product images) keeping position and crop.
- Placeholder replacement in notes, chart titles and SmartArt text.
- Hide/unhide slides; delete empty placeholders.

## Later

- Edit the existing embedded workbook instead of regenerating it, to keep
  workbook-level formatting.
- Text formatting helpers (bold/colour for replaced values).
- Table cell merge/split, column insert.
- Office 2016 charts (`cx:chart`: waterfall, treemap, ...).
- Rendering to PDF/PNG (separate crate; large effort).

## Not planned

- Legacy binary `.ppt`.
- Building decks from scratch (use python-pptx for that).
