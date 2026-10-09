# afterslides

Fill PowerPoint (`.pptx`) templates with data: replace placeholder text, fill
tables and native charts (including their embedded workbooks), and delete,
duplicate or reorder slides, without disturbing anything else in the file.

```rust,no_run
use afterslides::{Categories, ChartData, Presentation, Series};

let mut prs = Presentation::open("template.pptx")?;
prs.replace_text(&[("{{customer}}", "Acme")], true)?;

let slide = prs.slide_at(1)?;
let chart = prs.find_shape(slide, "Revenue Chart")?.expect("template has the chart");
prs.set_chart_data(chart, &ChartData::new(
    Categories::Labels(vec!["Q1".into(), "Q2".into()]),
    vec![Series::new("2026", vec![Some(1.5), Some(2.0)])],
))?;

prs.save("report.pptx")?;
# Ok::<(), afterslides::Error>(())
```

Most users use the Python package (`pip install afterslides`); this crate is
its engine and can be used on its own. See the
[project README](https://github.com/afterslides/afterslides) for the full
feature list.

Licensed under LGPL-3.0-or-later.
