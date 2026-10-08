//! Tables (`a:tbl` inside a graphic frame).

use crate::error::{Error, Result};
use crate::presentation::{Presentation, ShapeRef};
use crate::shape::kind_of;
use crate::text;
use crate::xml::{Element, Node, ns};

const A16: &str = "http://schemas.microsoft.com/office/drawing/2014/main";

fn tbl(frame: &Element) -> Option<&Element> {
    frame.path(&[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::A, "tbl")])
}

fn tbl_mut(frame: &mut Element) -> Option<&mut Element> {
    frame.path_mut(&[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::A, "tbl")])
}

fn not_a_table(shape: ShapeRef, el: &Element) -> Error {
    Error::Unsupported(format!(
        "shape {} is a {}, not a table",
        shape.id,
        kind_of(el).as_str()
    ))
}

fn rows(tbl: &Element) -> Vec<&Element> {
    tbl.children_named(ns::A, "tr").collect()
}

fn cell_text(tc: &Element) -> String {
    tc.child(ns::A, "txBody")
        .map(text::get_text)
        .unwrap_or_default()
}

fn set_cell_text(tc: &mut Element, value: &str) {
    if tc.child(ns::A, "txBody").is_none() {
        let body = text::new_body(tc, "txBody", tc);
        tc.children.insert(0, Node::Element(body));
    }
    text::set_text(tc.child_mut(ns::A, "txBody").expect("just ensured"), value);
}

/// Office 2016 tags rows and columns with ids that must stay unique within a
/// table; copies get fresh ones.
fn refresh_ext_id(el: &mut Element, local: &str, used: &mut Vec<u64>) {
    el.walk_mut(&mut |e| {
        if e.is(A16, local) && e.attr("val").is_some() {
            let next = used.iter().max().copied().unwrap_or(1_000_000_000) + 1;
            used.push(next);
            e.set_attr("val", next.to_string());
        }
    });
}

fn ext_ids(tbl: &Element, local: &str) -> Vec<u64> {
    let mut out = Vec::new();
    tbl.walk(&mut |e| {
        if e.is(A16, local)
            && let Some(v) = e.attr("val").and_then(|v| v.parse().ok())
        {
            out.push(v);
        }
    });
    out
}

/// Keeps the graphic frame's height in line with its rows so PowerPoint
/// doesn't draw stale selection handles.
fn sync_frame_height(frame: &mut Element) {
    let Some(tbl) = tbl(frame) else { return };
    let height: i64 = rows(tbl)
        .iter()
        .filter_map(|tr| tr.attr("h").and_then(|h| h.parse::<i64>().ok()))
        .sum();
    if let Some(ext) = frame.path_mut(&[(ns::P, "xfrm"), (ns::A, "ext")]) {
        ext.set_attr("cy", height.to_string());
    }
}

fn sync_frame_width(frame: &mut Element) {
    let Some(tbl) = tbl(frame) else { return };
    let width: i64 = tbl
        .child(ns::A, "tblGrid")
        .into_iter()
        .flat_map(|g| g.children_named(ns::A, "gridCol"))
        .filter_map(|c| c.attr("w").and_then(|w| w.parse::<i64>().ok()))
        .sum();
    if let Some(ext) = frame.path_mut(&[(ns::P, "xfrm"), (ns::A, "ext")]) {
        ext.set_attr("cx", width.to_string());
    }
}

impl Presentation {
    /// Cell texts, row by row. Merged-away cells are reported as empty strings
    /// so every row has the same length.
    pub fn table_values(&self, shape: ShapeRef) -> Result<Vec<Vec<String>>> {
        let el = self.shape_element(shape)?;
        let tbl = tbl(el).ok_or_else(|| not_a_table(shape, el))?;
        Ok(rows(tbl)
            .into_iter()
            .map(|tr| tr.children_named(ns::A, "tc").map(cell_text).collect())
            .collect())
    }

    /// `(rows, columns)` of a table.
    pub fn table_size(&self, shape: ShapeRef) -> Result<(usize, usize)> {
        let el = self.shape_element(shape)?;
        let tbl = tbl(el).ok_or_else(|| not_a_table(shape, el))?;
        let cols = tbl
            .child(ns::A, "tblGrid")
            .map_or(0, |g| g.children_named(ns::A, "gridCol").count());
        Ok((rows(tbl).len(), cols))
    }

    pub fn table_cell_text(&self, shape: ShapeRef, row: usize, col: usize) -> Result<String> {
        let values = self.table_values(shape)?;
        values
            .get(row)
            .and_then(|r| r.get(col))
            .cloned()
            .ok_or_else(|| {
                Error::InvalidArgument(format!("cell ({row}, {col}) is outside the table"))
            })
    }

    pub fn set_table_cell_text(
        &mut self,
        shape: ShapeRef,
        row: usize,
        col: usize,
        value: &str,
    ) -> Result<()> {
        self.with_shape_mut(shape, |el| {
            let err = not_a_table(shape, el);
            let tbl = tbl_mut(el).ok_or(err)?;
            let tc = tbl
                .children_named_mut(ns::A, "tr")
                .nth(row)
                .and_then(|tr| tr.children_named_mut(ns::A, "tc").nth(col))
                .ok_or_else(|| {
                    Error::InvalidArgument(format!("cell ({row}, {col}) is outside the table"))
                })?;
            set_cell_text(tc, value);
            Ok(())
        })
    }

    /// Inserts a copy of row `source` at position `at` (`at == rows` appends).
    /// The copy keeps all formatting; its text is cleared.
    pub fn insert_table_row(&mut self, shape: ShapeRef, source: usize, at: usize) -> Result<()> {
        self.with_shape_mut(shape, |el| {
            let err = not_a_table(shape, el);
            let tbl = tbl_mut(el).ok_or(err)?;
            let row_positions: Vec<usize> = tbl
                .children
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(ns::A, "tr")))
                .map(|(i, _)| i)
                .collect();
            let (&src_pos, n) = (
                row_positions.get(source).ok_or_else(|| {
                    Error::InvalidArgument(format!("row {source} is outside the table"))
                })?,
                row_positions.len(),
            );
            if at > n {
                return Err(Error::InvalidArgument(format!(
                    "cannot insert at row {at} of a table with {n} rows"
                )));
            }
            let mut used = ext_ids(tbl, "rowId");
            let Node::Element(mut copy) = tbl.children[src_pos].clone() else {
                unreachable!()
            };
            for tc in copy.children_named_mut(ns::A, "tc") {
                // A copied row must not continue a vertical merge from above
                // or start one into the row below.
                tc.remove_attr("rowSpan");
                tc.remove_attr("vMerge");
                set_cell_text(tc, "");
            }
            refresh_ext_id(&mut copy, "rowId", &mut used);
            let insert_pos = if at == n {
                row_positions.last().map_or(tbl.children.len(), |p| p + 1)
            } else {
                row_positions[at]
            };
            tbl.children.insert(insert_pos, Node::Element(copy));
            sync_frame_height(el);
            Ok(())
        })
    }

    pub fn delete_table_row(&mut self, shape: ShapeRef, row: usize) -> Result<()> {
        self.with_shape_mut(shape, |el| {
            let err = not_a_table(shape, el);
            let tbl = tbl_mut(el).ok_or(err)?;
            let pos = tbl
                .children
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(ns::A, "tr")))
                .map(|(i, _)| i)
                .nth(row)
                .ok_or_else(|| Error::InvalidArgument(format!("row {row} is outside the table")))?;
            tbl.children.remove(pos);
            sync_frame_height(el);
            Ok(())
        })
    }

    /// Removes a column; the table (and its frame) gets narrower by that
    /// column's width.
    pub fn delete_table_column(&mut self, shape: ShapeRef, col: usize) -> Result<()> {
        self.with_shape_mut(shape, |el| {
            let err = not_a_table(shape, el);
            let tbl = tbl_mut(el).ok_or(err)?;
            let grid = tbl
                .child_mut(ns::A, "tblGrid")
                .ok_or_else(|| Error::Package("table without a:tblGrid".into()))?;
            let pos = grid
                .children
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(ns::A, "gridCol")))
                .map(|(i, _)| i)
                .nth(col)
                .ok_or_else(|| {
                    Error::InvalidArgument(format!("column {col} is outside the table"))
                })?;
            grid.children.remove(pos);
            for tr in tbl.children_named_mut(ns::A, "tr") {
                let positions: Vec<usize> = tr
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(ns::A, "tc")))
                    .map(|(i, _)| i)
                    .collect();
                let Some(&cell_pos) = positions.get(col) else {
                    continue;
                };
                // A cell that spans into the removed column gets one shorter.
                for (k, &p) in positions[..col].iter().enumerate() {
                    if let Node::Element(tc) = &mut tr.children[p]
                        && let Some(span) =
                            tc.attr("gridSpan").and_then(|v| v.parse::<usize>().ok())
                        && k + span > col
                    {
                        if span > 2 {
                            tc.set_attr("gridSpan", (span - 1).to_string());
                        } else {
                            tc.remove_attr("gridSpan");
                        }
                    }
                }
                // If the removed cell starts a span, hand it to the next cell.
                if let Node::Element(tc) = &tr.children[cell_pos]
                    && let Some(span) = tc.attr("gridSpan").and_then(|v| v.parse::<usize>().ok())
                    && span > 1
                    && let Some(&next) = positions.get(col + 1)
                {
                    let span_left = span - 1;
                    let mut head = tc.clone();
                    if span_left > 1 {
                        head.set_attr("gridSpan", span_left.to_string());
                    } else {
                        head.remove_attr("gridSpan");
                    }
                    tr.children[next] = Node::Element(head);
                }
                tr.children.remove(cell_pos);
            }
            sync_frame_width(el);
            Ok(())
        })
    }

    /// Writes `data` into the table starting at `start_row`.
    ///
    /// With `resize`, rows are added (copies of the last row) or removed
    /// (from the end) so the table ends exactly after the data. Rows longer
    /// than the table is wide are an error; shorter rows leave the remaining
    /// cells untouched.
    pub fn fill_table(
        &mut self,
        shape: ShapeRef,
        data: &[Vec<String>],
        start_row: usize,
        resize: bool,
    ) -> Result<()> {
        let (rows, cols) = self.table_size(shape)?;
        if let Some((i, row)) = data.iter().enumerate().find(|(_, r)| r.len() > cols) {
            return Err(Error::InvalidArgument(format!(
                "row {i} has {} values but the table has {cols} columns",
                row.len()
            )));
        }
        let needed = start_row + data.len();
        if resize {
            if rows == 0 {
                return Err(Error::InvalidArgument("table has no rows to copy".into()));
            }
            for _ in rows..needed {
                let current = self.table_size(shape)?.0;
                self.insert_table_row(shape, current - 1, current)?;
            }
            for _ in needed.max(1)..rows {
                let current = self.table_size(shape)?.0;
                self.delete_table_row(shape, current - 1)?;
            }
        } else if needed > rows {
            return Err(Error::InvalidArgument(format!(
                "{} data rows starting at row {start_row} don't fit into {rows} rows; \
                 pass resize=true to grow the table",
                data.len()
            )));
        }

        self.with_shape_mut(shape, |el| {
            let err = not_a_table(shape, el);
            let tbl = tbl_mut(el).ok_or(err)?;
            for (tr, values) in tbl
                .children_named_mut(ns::A, "tr")
                .skip(start_row)
                .zip(data)
            {
                for (tc, value) in tr.children_named_mut(ns::A, "tc").zip(values) {
                    set_cell_text(tc, value);
                }
            }
            Ok(())
        })
    }
}
