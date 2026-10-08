# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may
contain breaking changes.

## [Unreleased]

### Added

- Open and save `.pptx` files from paths, bytes and streams; untouched parts
  are written back byte for byte.
- Find shapes by name, alt text and kind, including inside groups.
- Replace `{{placeholders}}` across runs, keeping formatting.
- Set shape text; delete shapes, releasing charts and media nobody uses.
- Fill tables, growing or shrinking them; insert/delete rows and columns.
- Read and replace data of category charts (bar, column, line, pie, doughnut,
  area, radar) and scatter/bubble charts, including the embedded workbook.
  Chart titles.
- Delete, duplicate and move slides, keeping sections, custom shows, notes and
  hyperlinks consistent.
- Python package with typed API, pandas DataFrame and date support.

### Fixed

- Pictures referenced through percent-encoded or differently cased
  relationship targets were deleted when cleaning up after a deletion.
- Shapes sharing an id with another shape on the same slide could be
  edited or deleted together; such ids are now made unique on open.
- Duplicated slides kept references to the original's modern comments and
  its `p14:creationId`.
- New chart series could reuse the index of a filtered (hidden) series.
- Combo charts now refuse a different number of series instead of putting
  all new series into the last plot.
- Infinite chart values are rejected instead of written into the chart.
- Setting text on a shape with an extension list produced invalid XML.
- Growing, shrinking or deleting rows of tables with vertically merged cells
  left merges pointing past the table.
- Concurrent saves to the same path could interfere through a shared temp
  file.
