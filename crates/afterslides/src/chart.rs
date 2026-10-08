//! Native charts (`c:chartSpace` parts) and their embedded workbooks.
//!
//! Replacing chart data means three things have to agree: the series in the
//! chart XML, the cached values PowerPoint draws from, and the embedded
//! workbook that opens when someone clicks "Edit Data". We rewrite all
//! three. New series are copies of the last series of the template, so they
//! inherit its formatting.

use std::collections::HashSet;

use rust_xlsxwriter::{Format, Workbook};

use crate::error::{Error, Result};
use crate::opc::{content_type, rel_type};
use crate::presentation::{Presentation, ShapeRef};
use crate::shape::kind_of;
use crate::text;
use crate::xml::{Document, Element, Node, ns};

/// Categories of a category chart (bar, column, line, pie, area, radar, ...).
#[derive(Debug, Clone, PartialEq)]
pub enum Categories {
    Labels(Vec<String>),
    /// Numeric categories, including dates as Excel serial numbers.
    Numbers(Vec<f64>),
}

impl Categories {
    pub fn len(&self) -> usize {
        match self {
            Categories::Labels(v) => v.len(),
            Categories::Numbers(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    /// One value per category; `None` leaves a gap.
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    pub categories: Categories,
    pub series: Vec<Series>,
}

/// A series of a scatter or bubble chart.
#[derive(Debug, Clone, PartialEq)]
pub struct XySeries {
    pub name: String,
    pub x: Vec<Option<f64>>,
    pub y: Vec<Option<f64>>,
    /// Bubble sizes; required for bubble charts, ignored otherwise.
    pub sizes: Option<Vec<Option<f64>>>,
}

/// Elements that precede `c:cat`/`c:xVal` in every series type of the schema.
const BEFORE_CAT: &[&str] = &[
    "idx",
    "order",
    "tx",
    "spPr",
    "invertIfNegative",
    "pictureOptions",
    "marker",
    "explosion",
    "dPt",
    "dLbls",
    "trendline",
    "errBars",
];

fn before(extra: &[&'static str]) -> Vec<&'static str> {
    BEFORE_CAT
        .iter()
        .copied()
        .chain(extra.iter().copied())
        .collect()
}

fn is_plot(e: &Element) -> bool {
    e.ns() == Some(ns::C) && e.local().ends_with("Chart")
}

fn is_xy_plot(e: &Element) -> bool {
    e.is(ns::C, "scatterChart") || e.is(ns::C, "bubbleChart")
}

fn plot_area(doc: &Document) -> Result<&Element> {
    doc.root
        .path(&[(ns::C, "chart"), (ns::C, "plotArea")])
        .ok_or_else(|| Error::Package("chart has no c:plotArea".into()))
}

fn plot_area_mut(doc: &mut Document) -> Result<&mut Element> {
    doc.root
        .path_mut(&[(ns::C, "chart"), (ns::C, "plotArea")])
        .ok_or_else(|| Error::Package("chart has no c:plotArea".into()))
}

fn all_series(plot_area: &Element) -> Vec<&Element> {
    plot_area
        .elements()
        .filter(|e| is_plot(e))
        .flat_map(|p| p.children_named(ns::C, "ser"))
        .collect()
}

// ---- reading caches -------------------------------------------------------

/// Points of the first cache or literal under `el` (`c:cat`, `c:val`, ...).
fn read_points(el: &Element) -> (Vec<Option<String>>, Option<String>) {
    let mut cache = None;
    el.walk(&mut |e| {
        if cache.is_none()
            && e.ns() == Some(ns::C)
            && matches!(
                e.local(),
                "strCache" | "numCache" | "strLit" | "numLit" | "lvl"
            )
        {
            cache = Some(e.clone());
        }
    });
    let Some(cache) = cache else {
        return (Vec::new(), None);
    };
    let count = cache
        .child(ns::C, "ptCount")
        .and_then(|c| c.attr("val"))
        .and_then(|v| v.parse::<usize>().ok());
    let pts: Vec<(usize, String)> = cache
        .children_named(ns::C, "pt")
        .filter_map(|pt| {
            let idx = pt.attr("idx")?.parse().ok()?;
            Some((
                idx,
                pt.child(ns::C, "v").map(Element::text).unwrap_or_default(),
            ))
        })
        .collect();
    let len = count.unwrap_or_else(|| pts.iter().map(|(i, _)| i + 1).max().unwrap_or(0));
    let mut out = vec![None; len];
    for (idx, v) in pts {
        if idx < len {
            out[idx] = Some(v);
        }
    }
    let format = cache
        .child(ns::C, "formatCode")
        .map(Element::text)
        .filter(|f| !f.is_empty());
    (out, format)
}

fn read_numbers(el: Option<&Element>) -> Vec<Option<f64>> {
    el.map(|el| {
        read_points(el)
            .0
            .into_iter()
            .map(|v| v.and_then(|s| s.trim().parse().ok()))
            .collect()
    })
    .unwrap_or_default()
}

fn series_name(ser: &Element) -> String {
    let Some(tx) = ser.child(ns::C, "tx") else {
        return String::new();
    };
    if let Some(v) = tx.child(ns::C, "v") {
        return v.text();
    }
    read_points(tx)
        .0
        .into_iter()
        .next()
        .flatten()
        .unwrap_or_default()
}

// ---- writing --------------------------------------------------------------

fn new_el(like: &Element, local: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
    let mut e = Element::new_like(like, local);
    for (k, v) in attrs {
        e.set_attr(k, *v);
    }
    e.children = children.into_iter().map(Node::Element).collect();
    e
}

fn text_el(like: &Element, local: &str, text: &str) -> Element {
    let mut e = Element::new_like(like, local);
    e.set_text(text);
    e
}

fn format_number(v: f64) -> String {
    // Integers without a trailing ".0", like Office writes them.
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn points(like: &Element, values: &[Option<String>]) -> Vec<Element> {
    let mut out = vec![new_el(
        like,
        "ptCount",
        &[("val", &values.len().to_string())],
        vec![],
    )];
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v {
            out.push(new_el(
                like,
                "pt",
                &[("idx", &i.to_string())],
                vec![text_el(like, "v", v)],
            ));
        }
    }
    out
}

/// Rewrites a data reference (`c:cat`, `c:val`, `c:xVal`, ...) in place.
///
/// Literal data stays literal; references get a fresh formula and cache.
fn write_data(
    holder: &mut Element,
    numeric: bool,
    values: &[Option<String>],
    formula: &str,
    default_format: &str,
) {
    let old_format = read_points(holder).1;
    let was_literal = holder
        .elements()
        .any(|e| e.is(ns::C, "strLit") || e.is(ns::C, "numLit"));
    let like = holder.clone();
    let mut cache_children = Vec::new();
    if numeric {
        let fmt = old_format.unwrap_or_else(|| default_format.to_string());
        cache_children.push(text_el(&like, "formatCode", &fmt));
    }
    cache_children.extend(points(&like, values));

    let content = match (was_literal, numeric) {
        (true, true) => new_el(&like, "numLit", &[], cache_children),
        (true, false) => new_el(&like, "strLit", &[], cache_children),
        (false, true) => new_el(
            &like,
            "numRef",
            &[],
            vec![
                text_el(&like, "f", formula),
                new_el(&like, "numCache", &[], cache_children),
            ],
        ),
        (false, false) => new_el(
            &like,
            "strRef",
            &[],
            vec![
                text_el(&like, "f", formula),
                new_el(&like, "strCache", &[], cache_children),
            ],
        ),
    };
    holder.children = vec![Node::Element(content)];
}

fn write_name(ser: &mut Element, name: &str, formula: &str) {
    let tx = ser.ensure_child("tx", &["idx", "order"]);
    if tx.child(ns::C, "v").is_some() {
        let v = tx.child_mut(ns::C, "v").expect("checked");
        v.set_text(name);
        return;
    }
    let like = tx.clone();
    tx.children = vec![Node::Element(new_el(
        &like,
        "strRef",
        &[],
        vec![
            text_el(&like, "f", formula),
            new_el(
                &like,
                "strCache",
                &[],
                points(&like, &[Some(name.to_string())]),
            ),
        ],
    ))];
}

/// Drops per-point formatting and labels that point past the new data.
fn prune_points(ser: &mut Element, len: usize) {
    let beyond = |e: &Element| {
        e.child(ns::C, "idx")
            .and_then(|i| i.attr("val"))
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|i| i >= len)
    };
    ser.children
        .retain(|n| !matches!(n, Node::Element(e) if e.is(ns::C, "dPt") && beyond(e)));
    if let Some(dlbls) = ser.child_mut(ns::C, "dLbls") {
        dlbls
            .children
            .retain(|n| !matches!(n, Node::Element(e) if e.is(ns::C, "dLbl") && beyond(e)));
    }
}

fn col_letter(mut col: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).expect("ASCII")
}

fn cell_ref(sheet: &str, col: usize, row: usize) -> String {
    format!("{sheet}!${}${}", col_letter(col), row + 1)
}

fn range_ref(sheet: &str, col: usize, first_row: usize, len: usize) -> String {
    let last = first_row + len.max(1) - 1;
    format!(
        "{sheet}!${c}${}:${c}${}",
        first_row + 1,
        last + 1,
        c = col_letter(col)
    )
}

/// The sheet name used in the template's formulas, as written (possibly quoted).
fn sheet_ref(plot_area: &Element) -> String {
    let mut found = None;
    plot_area.walk(&mut |e| {
        if found.is_none()
            && e.is(ns::C, "f")
            && let Some((sheet, _)) = e.text().rsplit_once('!')
        {
            found = Some(sheet.to_string());
        }
    });
    found.unwrap_or_else(|| "Sheet1".to_string())
}

fn unquote_sheet(sheet: &str) -> String {
    sheet
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .map(|s| s.replace("''", "'"))
        .unwrap_or_else(|| sheet.to_string())
}

/// Makes the series list match `count`: extra series are copies of the last
/// one, surplus series are removed from the end. Returns `(plot, ser)` child
/// index pairs in order.
fn resize_series(plot_area: &mut Element, count: usize) -> Result<Vec<(usize, usize)>> {
    let locate = |pa: &Element| -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (pi, node) in pa.children.iter().enumerate() {
            if let Node::Element(plot) = node
                && is_plot(plot)
            {
                for (si, s) in plot.children.iter().enumerate() {
                    if matches!(s, Node::Element(e) if e.is(ns::C, "ser")) {
                        out.push((pi, si));
                    }
                }
            }
        }
        out
    };
    let mut locs = locate(plot_area);
    let Some(&(last_plot, last_ser)) = locs.last() else {
        return Err(Error::Unsupported(
            "chart has no series to copy formatting from; add at least one in the template".into(),
        ));
    };

    if count > locs.len() {
        let mut used_idx: Vec<u64> = Vec::new();
        let mut used_order: Vec<u64> = Vec::new();
        let mut used_guids: HashSet<String> = HashSet::new();
        for (pi, si) in &locs {
            let ser = child_el(child_el(plot_area, *pi), *si);
            let num = |name| {
                ser.child(ns::C, name)
                    .and_then(|e| e.attr("val"))
                    .and_then(|v| v.parse::<u64>().ok())
            };
            used_idx.extend(num("idx"));
            used_order.extend(num("order"));
            ser.walk(&mut |e| {
                if e.local() == "uniqueId"
                    && let Some(v) = e.attr("val")
                {
                    used_guids.insert(v.to_string());
                }
            });
        }
        let template = child_el(child_el(plot_area, last_plot), last_ser).clone();
        let plot = child_el_mut(plot_area, last_plot);
        for (insert_at, _) in (last_ser + 1..).zip(locs.len()..count) {
            let mut copy = template.clone();
            let idx = used_idx.iter().max().map_or(0, |m| m + 1);
            let order = used_order.iter().max().map_or(0, |m| m + 1);
            used_idx.push(idx);
            used_order.push(order);
            if let Some(e) = copy.child_mut(ns::C, "idx") {
                e.set_attr("val", idx.to_string());
            }
            if let Some(e) = copy.child_mut(ns::C, "order") {
                e.set_attr("val", order.to_string());
            }
            copy.walk_mut(&mut |e| {
                if e.local() == "uniqueId" && e.attr("val").is_some() {
                    let guid = (0u64..)
                        .map(|n| format!("{{AF7E0000-0000-4000-8000-{:012X}}}", idx * 1000 + n))
                        .find(|g| !used_guids.contains(g))
                        .expect("unbounded");
                    used_guids.insert(guid.clone());
                    e.set_attr("val", guid);
                }
            });
            plot.children.insert(insert_at, Node::Element(copy));
        }
        locs = locate(plot_area);
    }

    while locs.len() > count {
        let (pi, si) = locs.pop().expect("non-empty");
        child_el_mut(plot_area, pi).children.remove(si);
    }
    Ok(locate(plot_area))
}

fn child_el(el: &Element, i: usize) -> &Element {
    match &el.children[i] {
        Node::Element(e) => e,
        _ => unreachable!("index points at an element"),
    }
}

fn child_el_mut(el: &mut Element, i: usize) -> &mut Element {
    match &mut el.children[i] {
        Node::Element(e) => e,
        _ => unreachable!("index points at an element"),
    }
}

fn wb_err(e: rust_xlsxwriter::XlsxError) -> Error {
    Error::Workbook(e.to_string())
}

/// One column of the embedded workbook.
struct Column {
    header: Option<String>,
    cells: Vec<Option<Cell>>,
    format: Option<String>,
}

enum Cell {
    Text(String),
    Number(f64),
}

fn build_workbook(sheet: &str, columns: &[Column]) -> Result<Vec<u8>> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name(sheet).map_err(wb_err)?;
    for (c, column) in columns.iter().enumerate() {
        let c = u16::try_from(c).map_err(|_| Error::InvalidArgument("too many series".into()))?;
        if let Some(h) = &column.header {
            ws.write_string(0, c, h).map_err(wb_err)?;
        }
        let format = column
            .format
            .as_deref()
            .filter(|f| *f != "General")
            .map(|f| Format::new().set_num_format(f));
        for (r, cell) in column.cells.iter().enumerate() {
            let r =
                u32::try_from(r + 1).map_err(|_| Error::InvalidArgument("too many rows".into()))?;
            match (cell, &format) {
                (Some(Cell::Text(s)), _) => {
                    ws.write_string(r, c, s).map_err(wb_err)?;
                }
                (Some(Cell::Number(n)), Some(f)) => {
                    ws.write_number_with_format(r, c, *n, f).map_err(wb_err)?;
                }
                (Some(Cell::Number(n)), None) => {
                    ws.write_number(r, c, *n).map_err(wb_err)?;
                }
                (None, _) => {}
            }
        }
    }
    wb.save_to_buffer().map_err(wb_err)
}

fn number_cells(values: &[Option<f64>]) -> Vec<Option<Cell>> {
    values.iter().map(|v| v.map(Cell::Number)).collect()
}

fn number_strings(values: &[Option<f64>]) -> Vec<Option<String>> {
    values.iter().map(|v| v.map(format_number)).collect()
}

impl Presentation {
    /// Part name of the chart behind a chart shape.
    pub fn chart_part(&self, shape: ShapeRef) -> Result<String> {
        let el = self.shape_element(shape)?;
        let rid = el
            .path(&[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::C, "chart")])
            .and_then(|c| c.attr_ns(ns::R, "id"))
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "shape {} is a {}, not a chart",
                    shape.id,
                    kind_of(el).as_str()
                ))
            })?;
        let slide_part = self.slide_part(shape.slide)?;
        self.pkg.resolve(&slide_part, rid)
    }

    /// Plot types in the chart, e.g. `["barChart", "lineChart"]` for a combo chart.
    pub fn chart_types(&self, shape: ShapeRef) -> Result<Vec<String>> {
        let part = self.chart_part(shape)?;
        let pa = plot_area(self.pkg.xml(&part)?)?;
        Ok(pa
            .elements()
            .filter(|e| is_plot(e))
            .map(|e| e.local().to_string())
            .collect())
    }

    /// Data of a category chart as PowerPoint last cached it.
    pub fn chart_data(&self, shape: ShapeRef) -> Result<ChartData> {
        let part = self.chart_part(shape)?;
        let pa = plot_area(self.pkg.xml(&part)?)?;
        let series = all_series(pa);
        let categories = match series.first().and_then(|s| s.child(ns::C, "cat")) {
            Some(cat) => {
                let numeric = cat
                    .elements()
                    .any(|e| e.is(ns::C, "numRef") || e.is(ns::C, "numLit"));
                let (pts, _) = read_points(cat);
                if numeric {
                    Categories::Numbers(
                        pts.into_iter()
                            .map(|p| p.and_then(|s| s.parse().ok()).unwrap_or(f64::NAN))
                            .collect(),
                    )
                } else {
                    Categories::Labels(pts.into_iter().map(Option::unwrap_or_default).collect())
                }
            }
            None => Categories::Labels(Vec::new()),
        };
        Ok(ChartData {
            categories,
            series: series
                .into_iter()
                .map(|ser| Series {
                    name: series_name(ser),
                    values: read_numbers(ser.child(ns::C, "val")),
                })
                .collect(),
        })
    }

    /// Series of a scatter or bubble chart.
    pub fn chart_xy_data(&self, shape: ShapeRef) -> Result<Vec<XySeries>> {
        let part = self.chart_part(shape)?;
        let pa = plot_area(self.pkg.xml(&part)?)?;
        Ok(all_series(pa)
            .into_iter()
            .map(|ser| XySeries {
                name: series_name(ser),
                x: read_numbers(ser.child(ns::C, "xVal")),
                y: read_numbers(ser.child(ns::C, "yVal")),
                sizes: ser
                    .child(ns::C, "bubbleSize")
                    .map(|b| read_numbers(Some(b))),
            })
            .collect())
    }

    /// Replaces the data of a category chart.
    ///
    /// Series beyond the template's are copies of its last series; surplus
    /// template series are removed. For combo charts, series are filled in
    /// document order across plots.
    pub fn set_chart_data(&mut self, shape: ShapeRef, data: &ChartData) -> Result<()> {
        let n = data.categories.len();
        for s in &data.series {
            if s.values.len() > n {
                return Err(Error::InvalidArgument(format!(
                    "series {:?} has {} values but there are only {n} categories",
                    s.name,
                    s.values.len()
                )));
            }
        }
        let part = self.chart_part(shape)?;
        let doc = self.pkg.xml(&part)?;
        if plot_area(doc)?.elements().any(is_xy_plot) {
            return Err(Error::Unsupported(
                "this is a scatter or bubble chart; use set_chart_xy_data".into(),
            ));
        }

        let doc = self.pkg.xml_mut(&part)?;
        let pa = plot_area_mut(doc)?;
        let sheet = sheet_ref(pa);
        let locs = resize_series(pa, data.series.len())?;

        let (cat_numeric, cat_strings, cat_cells): (bool, Vec<Option<String>>, Vec<Option<Cell>>) =
            match &data.categories {
                Categories::Labels(v) => (
                    false,
                    v.iter().cloned().map(Some).collect(),
                    v.iter().cloned().map(|s| Some(Cell::Text(s))).collect(),
                ),
                Categories::Numbers(v) => (
                    true,
                    v.iter().map(|x| Some(format_number(*x))).collect(),
                    v.iter().map(|x| Some(Cell::Number(*x))).collect(),
                ),
            };

        let mut columns = vec![Column {
            header: None,
            cells: cat_cells,
            format: None,
        }];
        for (k, ((pi, si), series)) in locs.iter().zip(&data.series).enumerate() {
            let col = k + 1;
            let ser = child_el_mut(child_el_mut(pa, *pi), *si);
            write_name(ser, &series.name, &cell_ref(&sheet, col, 0));
            if cat_numeric && columns[0].format.is_none() {
                columns[0].format = ser.child(ns::C, "cat").and_then(|c| read_points(c).1);
            }
            let cat = ser.ensure_child("cat", BEFORE_CAT);
            write_data(
                cat,
                cat_numeric,
                &cat_strings,
                &range_ref(&sheet, 0, 1, n),
                "General",
            );

            let mut values = series.values.clone();
            values.resize(n, None);
            let val = ser.ensure_child("val", &before(&["cat"]));
            write_data(
                val,
                true,
                &number_strings(&values),
                &range_ref(&sheet, col, 1, n),
                "General",
            );
            let format = read_points(val).1;
            prune_points(ser, n);
            columns.push(Column {
                header: Some(series.name.clone()),
                cells: number_cells(&values),
                format,
            });
        }
        self.write_embedded_workbook(&part, &unquote_sheet(&sheet), &columns)
    }

    /// Replaces the data of a scatter or bubble chart.
    pub fn set_chart_xy_data(&mut self, shape: ShapeRef, series: &[XySeries]) -> Result<()> {
        let part = self.chart_part(shape)?;
        let doc = self.pkg.xml(&part)?;
        let pa = plot_area(doc)?;
        if !pa.elements().any(is_xy_plot) {
            return Err(Error::Unsupported(
                "this is not a scatter or bubble chart; use set_chart_data".into(),
            ));
        }
        let bubble = pa.elements().any(|e| e.is(ns::C, "bubbleChart"));
        for s in series {
            if s.x.len() != s.y.len() {
                return Err(Error::InvalidArgument(format!(
                    "series {:?} has {} x values but {} y values",
                    s.name,
                    s.x.len(),
                    s.y.len()
                )));
            }
            if bubble && s.sizes.as_ref().is_none_or(|b| b.len() != s.x.len()) {
                return Err(Error::InvalidArgument(format!(
                    "series {:?} needs one bubble size per point",
                    s.name
                )));
            }
        }

        let doc = self.pkg.xml_mut(&part)?;
        let pa = plot_area_mut(doc)?;
        let sheet = sheet_ref(pa);
        let locs = resize_series(pa, series.len())?;
        let width = if bubble { 3 } else { 2 };
        let mut columns = Vec::new();
        for (k, ((pi, si), s)) in locs.iter().zip(series).enumerate() {
            let (xc, yc, bc) = (k * width, k * width + 1, k * width + 2);
            let n = s.x.len();
            let ser = child_el_mut(child_el_mut(pa, *pi), *si);
            write_name(ser, &s.name, &cell_ref(&sheet, yc, 0));
            let x = ser.ensure_child("xVal", BEFORE_CAT);
            write_data(
                x,
                true,
                &number_strings(&s.x),
                &range_ref(&sheet, xc, 1, n),
                "General",
            );
            let x_format = read_points(x).1;
            let y = ser.ensure_child("yVal", &before(&["xVal"]));
            write_data(
                y,
                true,
                &number_strings(&s.y),
                &range_ref(&sheet, yc, 1, n),
                "General",
            );
            let y_format = read_points(y).1;
            columns.push(Column {
                header: Some("X".into()),
                cells: number_cells(&s.x),
                format: x_format,
            });
            columns.push(Column {
                header: Some(s.name.clone()),
                cells: number_cells(&s.y),
                format: y_format,
            });
            if bubble {
                let sizes = s.sizes.as_deref().unwrap_or_default();
                let b = ser.ensure_child("bubbleSize", &before(&["xVal", "yVal"]));
                write_data(
                    b,
                    true,
                    &number_strings(sizes),
                    &range_ref(&sheet, bc, 1, n),
                    "General",
                );
                columns.push(Column {
                    header: Some("Size".into()),
                    cells: number_cells(sizes),
                    format: None,
                });
            }
            prune_points(ser, n);
        }
        self.write_embedded_workbook(&part, &unquote_sheet(&sheet), &columns)
    }

    /// Writes a fresh workbook into the chart's embedded package, if it has one.
    fn write_embedded_workbook(
        &mut self,
        chart_part: &str,
        sheet: &str,
        columns: &[Column],
    ) -> Result<()> {
        let doc = self.pkg.xml(chart_part)?;
        let Some(rid) = doc
            .root
            .child(ns::C, "externalData")
            .and_then(|e| e.attr_ns(ns::R, "id"))
            .map(str::to_string)
        else {
            return Ok(());
        };
        let Some((rel, target)) = self
            .pkg
            .targets(chart_part)
            .into_iter()
            .find(|(r, _)| r.id == rid)
        else {
            // Linked to an external file; nothing we can update.
            return Ok(());
        };
        let is_xlsx = rel.rel_type == rel_type::PACKAGE
            && (self.pkg.content_type(&target).as_deref() == Some(content_type::XLSX)
                || target.to_ascii_lowercase().ends_with(".xlsx"));
        if !is_xlsx || !self.pkg.has_part(&target) {
            return Ok(());
        }
        let bytes = build_workbook(sheet, columns)?;
        self.pkg.set_raw(&target, bytes)
    }

    /// Chart title text, if the chart has a title with its own text.
    pub fn chart_title(&self, shape: ShapeRef) -> Result<Option<String>> {
        let part = self.chart_part(shape)?;
        let doc = self.pkg.xml(&part)?;
        let Some(tx) = doc
            .root
            .path(&[(ns::C, "chart"), (ns::C, "title"), (ns::C, "tx")])
        else {
            return Ok(None);
        };
        if let Some(rich) = tx.child(ns::C, "rich") {
            return Ok(Some(text::get_text(rich)));
        }
        Ok(read_points(tx).0.into_iter().next().flatten())
    }

    /// Sets the chart title. The chart must have a title in the template;
    /// its formatting is kept.
    pub fn set_chart_title(&mut self, shape: ShapeRef, title: &str) -> Result<()> {
        let part = self.chart_part(shape)?;
        let doc = self.pkg.xml_mut(&part)?;
        let a_prefix = doc.prefix_for(ns::A).map(str::to_string);
        let chart = doc
            .root
            .child_mut(ns::C, "chart")
            .ok_or_else(|| Error::Package("chart part has no c:chart".into()))?;
        let Some(title_el) = chart.child_mut(ns::C, "title") else {
            return Err(Error::Unsupported(
                "chart has no title; add one in the template".into(),
            ));
        };
        let tx = title_el.ensure_child("tx", &[]);
        if tx.child(ns::C, "rich").is_none() {
            // Title text from a cell reference or none at all: switch to rich text.
            let a = |local: &str| Element {
                name: format!("{}:{local}", a_prefix.as_deref().unwrap_or("a")),
                ns: Some(ns::A.into()),
                attrs: Vec::new(),
                children: Vec::new(),
            };
            let mut rich = Element::new_like(tx, "rich");
            rich.children = vec![
                Node::Element(a("bodyPr")),
                Node::Element(a("lstStyle")),
                Node::Element(a("p")),
            ];
            tx.children = vec![Node::Element(rich)];
        }
        text::set_text(tx.child_mut(ns::C, "rich").expect("ensured"), title);
        if let Some(deleted) = chart.child_mut(ns::C, "autoTitleDeleted") {
            deleted.set_attr("val", "0");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
        assert_eq!(col_letter(701), "ZZ");
        assert_eq!(col_letter(702), "AAA");
    }

    #[test]
    fn refs() {
        assert_eq!(cell_ref("Sheet1", 1, 0), "Sheet1!$B$1");
        assert_eq!(range_ref("'My data'", 2, 1, 3), "'My data'!$C$2:$C$4");
        assert_eq!(unquote_sheet("'It''s'"), "It's");
    }

    #[test]
    fn numbers() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(0.1 + 0.2), "0.30000000000000004");
    }
}
