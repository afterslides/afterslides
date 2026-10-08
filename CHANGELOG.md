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
