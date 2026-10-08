# Migrating from a commercial PPTX library

Report generators built on commercial PPTX libraries tend to use the same
dozen calls: open a template, find shapes, replace text, fill tables and
charts, drop what has no data, save. This page maps those patterns to
afterslides.

| Task | Typical commercial API | afterslides |
| --- | --- | --- |
| Open a template | `new Presentation(path)` | `Presentation(path)` |
| Save | `pres.Save(path, SaveFormat.Pptx)` | `prs.save(path)` |
| Slide by index | `pres.Slides[i]` | `prs.slides[i]` |
| Find shape by name | loop over `slide.Shapes`, compare `Name` | `slide.shape(name)` / `prs.shape(name)` |
| Find by alt text | compare `AlternativeText` | `prs.find_shapes(alt_text=...)` |
| Replace text | loop paragraphs/portions | `prs.replace_text({...})` |
| Set shape text | `autoShape.TextFrame.Text = ...` | `shape.text = ...` |
| Table cell | `table[col, row].TextFrame.Text` | `table[row, col] = ...` (row first!) |
| Add table row | `table.Rows.AddClone(row, false)` | `table.insert_row(copy_of=-1)` or `table.fill(rows)` |
| Remove table row | `table.Rows.RemoveAt(i, false)` | `table.delete_row(i)` |
| Chart workbook | `chart.ChartData.ChartDataWorkbook` + cell writes | `chart.replace_data(categories, series)` |
| Clear chart series | `chart.ChartData.Series.Clear()` | implicit in `replace_data` |
| Add series | `Series.Add(...)` + `DataPoints.AddDataPointForBarSeries(...)` | pass more series to `replace_data` |
| Chart title | `chart.ChartTitle.AddTextFrameForOverriding(...)` | `chart.title = ...` |
| Remove shape | `slide.Shapes.Remove(shape)` | `shape.delete()` |
| Remove slide | `pres.Slides.Remove(slide)` | `slide.delete()` |
| Clone slide | `pres.Slides.InsertClone(i, slide)` | `slide.duplicate(position=i)` |
| Reorder | `pres.Slides.Reorder(i, slide)` | `slide.move_to(i)` |

## Differences to keep in mind

- **Row first.** Table cells are addressed `table[row, column]`.
- **Charts are replaced as a whole.** Instead of editing workbook cells and
  re-binding series, pass the complete new data to `replace_data`. Number
  formats and series formatting come from the template.
- **No license file, no evaluation watermark.**
- **No rendering.** Convert to PDF with LibreOffice if you need it:
  `soffice --headless --convert-to pdf report.pptx`.
- **Untouched content stays byte-identical.** Some commercial libraries
  rewrite every part on save; afterslides only rewrites what you changed, so
  diffs between template and output stay small.

If you rely on a call that has no equivalent here, please open an issue with
the snippet; it helps us prioritize.
