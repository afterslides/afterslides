# afterslides

**Fill PowerPoint templates with data.** Design the deck in PowerPoint, mark
the spots with shape names or `{{placeholders}}`, and let afterslides put in
the text, table rows and chart series. Slides and shapes that have no data
can be removed.

afterslides is written in Rust with Python bindings. It is meant as an open
source replacement for commercial PPTX libraries in report-generation jobs.

```python
from afterslides import Presentation

prs = Presentation("quarterly-template.pptx")

prs.replace_text({"{{customer}}": "Acme Corp", "{{quarter}}": "Q3 2026"})

prs.shape("Revenue Chart").chart.replace_data(
    ["Jul", "Aug", "Sep"],
    {"2025": [1.2, 1.4, 1.1], "2026": [1.5, 1.7, 1.9]},
)

prs.shape("Top Customers").table.fill(
    [
        ["Globex", "1.2M", "31%"],
        ["Initech", "0.9M", "24%"],
    ]
)

risks = []  # nothing to report this quarter
if not risks:
    prs.shape("Risk Matrix").slide.delete()

prs.save("report.pptx")
```

## Features

- **Text**: replace `{{placeholders}}` anywhere on a slide, also when
  PowerPoint split them across differently formatted runs. Formatting is kept.
- **Charts**: replace categories and series of bar, column, line, pie,
  doughnut, area, radar, scatter and bubble charts. The embedded workbook is
  rewritten too, so *Edit Data* in PowerPoint shows the new numbers. Extra
  series take over the template's formatting; surplus ones are removed.
  Accepts dicts, lists of `Series`, or a pandas `DataFrame`. Dates work as
  categories.
- **Tables**: fill rows; the table grows (copying the formatting of its last
  row) or shrinks to fit. Insert and delete rows and columns.
- **Shapes**: find by name (as shown in the Selection Pane), alt text or kind,
  including inside groups. Set text, delete.
- **Slides**: delete, duplicate (with independent chart copies, e.g. one slide
  per region), reorder. Sections, notes, hyperlinks and unused media are kept
  consistent so PowerPoint never asks to repair the file.
- **Faithful**: everything you don't touch is written back byte for byte.
- **Fast and lean**: filling a small template (open, replace text, chart and
  table data, delete a slide, save) takes about 4 ms. No .NET, Java or
  LibreOffice needed at runtime.

What it does **not** do: render slides to PDF or images, build decks from
scratch, or read legacy `.ppt` files. For PDFs, convert the result with
LibreOffice (`soffice --headless --convert-to pdf report.pptx`).

## Installation

```sh
pip install afterslides
```

(Not on PyPI yet; until the first release, build from source with
`pip install git+https://github.com/afterslides/afterslides`, which needs a
Rust toolchain.)

Wheels are built for Linux, macOS and Windows (Python 3.9+, one `abi3` wheel
per platform). Rust users can depend on the
[`afterslides`](crates/afterslides) crate directly.

## Template conventions

afterslides doesn't impose a templating language. Two conventions work well:

1. **Name the shapes** you want to fill (Home → Arrange → Selection Pane in
   PowerPoint) and look them up with `prs.shape("Revenue Chart")` or
   `slide.shape(...)`.
2. **Write placeholders** like `{{customer}}` into text boxes and table cells
   and call `prs.replace_text({...})`. Any delimiter works; afterslides just
   replaces strings.

The alt text of a shape is a good place for extra markers
(`prs.find_shapes(alt_text="optional")`).

## Guide

### Opening and saving

```python
prs = Presentation("template.pptx")  # path, bytes or a binary stream
prs.save("out.pptx")  # path or writable binary stream
data = prs.to_bytes()
```

Saving to a path writes a temporary file first, so overwriting the template
is safe.

### Slides and shapes

```python
for slide in prs.slides:
    for shape in slide.shapes:
        print(slide.index, shape.name, shape.kind, shape.text)

slide = prs.slides[2]
chart_shape = slide.shape("Revenue Chart")
logo = prs.find_shapes(kind="picture")[0]

copy = slide.duplicate()  # inserted right after the original
copy.move_to(0)
slide.delete()
```

Slide and shape objects are handles: they stay valid when other slides are
added, moved or deleted, and raise `NotFoundError` once their own slide or
shape is gone.

### Charts

```python
chart = prs.shape("Revenue Chart").chart
chart.categories  # ['Q1', 'Q2', 'Q3', 'Q4']
chart.series  # [Series(name='2025', values=[...]), ...]

chart.replace_data(["Q1", "Q2"], {"Plan": [10, 12], "Actual": [11, None]})
chart.replace_data(df)  # pandas: index -> categories, columns -> series
chart.title = "Revenue 2026"

scatter = prs.shape("Scatter").chart
scatter.replace_xy_data([XySeries("Run 1", x=[1, 2, 3], y=[2.0, 3.5, 3.1])])
```

`None` (or NaN) leaves a gap.

Multi-level categories are tuples, outermost level first; a pandas
`MultiIndex` works the same way:

```python
chart.replace_data(
    [("2025", "Q3"), ("2025", "Q4"), ("2026", "Q1")],
    {"Revenue": [1.2, 1.4, 1.1]},
)
```

In **combo charts** (say, columns plus a line) each series belongs to a plot.
`chart.types` lists the plots, and `Series.plot` picks one; each plot copies
the formatting of its own template series:

```python
chart.types               # ['barChart', 'lineChart']
chart.replace_data(
    ["Q1", "Q2", "Q3"],
    [
        Series("Revenue 2025", [10, 12, 9], plot=0),
        Series("Revenue 2026", [11, 14, 10], plot=0),
        Series("Margin", [0.21, 0.24, 0.19], plot=1, number_format="0%"),
    ],
)
```

`Series.number_format` overrides the template's number format. Date
categories get a date format if the template has none. Number formats from the template, such as
`0%` or `#,##0.00`, are kept and also applied in the embedded workbook.

### Tables

```python
table = prs.shape("Top Customers").table
table.fill(rows)  # starts below the header row; resizes
table.fill(rows, start_row=0)  # no header
table[0, 1] = "Revenue (EUR)"
table.delete_column(-1)
```

### Errors

All exceptions derive from `afterslides.AfterslidesError`:
`PackageError` (not a valid .pptx), `NotFoundError` (also a `LookupError`),
`InvalidArgumentError` (also a `ValueError`) and `UnsupportedError`.

## Coming from a commercial library?

See [docs/migrating.md](docs/migrating.md) for a mapping of the usual
template-filling calls.

## Status

Alpha. The API may still change before 1.0. See [CHANGELOG.md](CHANGELOG.md)
and the [roadmap](docs/roadmap.md). Bug reports with an (anonymized) template
that shows the problem are very welcome.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). In short: `uv`, `cargo`, small
commits, tests that read our output back with an independent implementation.

## License

afterslides is licensed under the
[GNU Lesser General Public License v3.0 or later](LICENSE.md). You can use it
from proprietary applications; changes to afterslides itself must be shared
under the same license.

Copyright © 2026 Andreas Kluth and the afterslides contributors.
