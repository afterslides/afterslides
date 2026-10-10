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
#[non_exhaustive]
pub enum Categories {
    Labels(Vec<String>),
    /// Numeric categories, written with the template's number format.
    Numbers(Vec<f64>),
    /// Dates as Excel serial numbers. Like `Numbers`, but if the template has
    /// no number format for its categories, a date format is used.
    Dates(Vec<f64>),
    /// Multi-level categories (for example year, then quarter). One entry per
    /// category, outermost level first; all entries have the same depth.
    Levels(Vec<Vec<String>>),
}

impl Categories {
    pub fn len(&self) -> usize {
        match self {
            Categories::Labels(v) => v.len(),
            Categories::Numbers(v) | Categories::Dates(v) => v.len(),
            Categories::Levels(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of category levels (1 unless `Levels`).
    pub fn depth(&self) -> usize {
        match self {
            Categories::Levels(v) => v.first().map_or(1, Vec::len).max(1),
            _ => 1,
        }
    }
}

/// A series of a category chart.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Series {
    pub name: String,
    /// One value per category; `None` leaves a gap.
    pub values: Vec<Option<f64>>,
    /// Index of the plot (see [`Presentation::chart_types`]) the series
    /// belongs to. Only needed for combo charts, where it decides whether a
    /// series is drawn as, say, a column or a line.
    pub plot: Option<usize>,
    /// Number format for the values, e.g. `0.0%`. `None` keeps the template's.
    pub number_format: Option<String>,
}

impl Series {
    pub fn new(name: impl Into<String>, values: Vec<Option<f64>>) -> Series {
        Series {
            name: name.into(),
            values,
            plot: None,
            number_format: None,
        }
    }

    pub fn with_plot(mut self, plot: usize) -> Series {
        self.plot = Some(plot);
        self
    }

    pub fn with_number_format(mut self, format: impl Into<String>) -> Series {
        self.number_format = Some(format.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ChartData {
    pub categories: Categories,
    pub series: Vec<Series>,
}

impl ChartData {
    pub fn new(categories: Categories, series: Vec<Series>) -> ChartData {
        ChartData { categories, series }
    }
}

/// A series of a scatter or bubble chart.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XySeries {
    pub name: String,
    pub x: Vec<Option<f64>>,
    pub y: Vec<Option<f64>>,
    /// Bubble sizes; required for bubble charts, ignored otherwise.
    pub sizes: Option<Vec<Option<f64>>>,
}

impl XySeries {
    pub fn new(name: impl Into<String>, x: Vec<Option<f64>>, y: Vec<Option<f64>>) -> XySeries {
        XySeries {
            name: name.into(),
            x,
            y,
            sizes: None,
        }
    }

    pub fn with_sizes(mut self, sizes: Vec<Option<f64>>) -> XySeries {
        self.sizes = Some(sizes);
        self
    }
}

/// Date format for date categories when the template has none.
const DATE_FORMAT: &str = "yyyy\\-mm\\-dd";

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
pub(crate) fn read_points(el: &Element) -> (Vec<Option<String>>, Option<String>) {
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

pub(crate) fn read_numbers(el: Option<&Element>) -> Vec<Option<f64>> {
    el.map(|el| {
        read_points(el)
            .0
            .into_iter()
            .map(|v| v.and_then(|s| s.trim().parse().ok()))
            .collect()
    })
    .unwrap_or_default()
}

pub(crate) fn series_name(ser: &Element) -> String {
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

/// How to format numbers in a cache: an explicit format, the template's
/// format with a fallback, or the template's format falling back to
/// `General`.
enum NumberFormat<'a> {
    Explicit(&'a str),
    TemplateOr(&'a str),
}

/// Rewrites a data reference (`c:cat`, `c:val`, `c:xVal`, ...) in place.
///
/// Literal data stays literal; references get a fresh formula and cache.
/// Returns the number format written (for numeric data).
fn write_data(
    holder: &mut Element,
    numeric: bool,
    values: &[Option<String>],
    formula: &str,
    format: NumberFormat<'_>,
) -> Option<String> {
    let old_format = read_points(holder).1.filter(|f| f != "General");
    let was_literal = holder
        .elements()
        .any(|e| e.is(ns::C, "strLit") || e.is(ns::C, "numLit"));
    let like = holder.clone();
    let mut cache_children = Vec::new();
    let fmt = numeric.then(|| match format {
        NumberFormat::Explicit(f) => f.to_string(),
        NumberFormat::TemplateOr(fallback) => old_format.unwrap_or_else(|| fallback.to_string()),
    });
    if let Some(fmt) = &fmt {
        cache_children.push(text_el(&like, "formatCode", fmt));
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
    fmt
}

/// Writes multi-level categories as `c:multiLvlStrRef`. Levels are stored
/// innermost first; outer levels only carry a label where a group starts.
fn write_levels(holder: &mut Element, levels: &[Vec<String>], formula: &str) {
    let like = holder.clone();
    let depth = levels.first().map_or(0, Vec::len);
    let mut cache = vec![new_el(
        &like,
        "ptCount",
        &[("val", &levels.len().to_string())],
        vec![],
    )];
    for level in (0..depth).rev() {
        let pts = group_starts(levels, level)
            .into_iter()
            .map(|(i, label)| {
                new_el(
                    &like,
                    "pt",
                    &[("idx", &i.to_string())],
                    vec![text_el(&like, "v", label)],
                )
            })
            .collect();
        cache.push(new_el(&like, "lvl", &[], pts));
    }
    holder.children = vec![Node::Element(new_el(
        &like,
        "multiLvlStrRef",
        &[],
        vec![
            text_el(&like, "f", formula),
            new_el(&like, "multiLvlStrCache", &[], cache),
        ],
    ))];
}

/// Positions where the label of `level` starts a new group: the label
/// changes, or any outer level does. The innermost level labels every
/// category.
fn group_starts(levels: &[Vec<String>], level: usize) -> Vec<(usize, &str)> {
    let depth = levels.first().map_or(0, Vec::len);
    levels
        .iter()
        .enumerate()
        .filter(|(i, row)| {
            level + 1 == depth || *i == 0 || levels[i - 1][..=level] != row[..=level]
        })
        .map(|(i, row)| (i, row[level].as_str()))
        .collect()
}

/// Reads `c:multiLvlStrRef` back into one label path per category.
fn read_levels(cat: &Element) -> Option<Vec<Vec<String>>> {
    let cache = cat
        .child(ns::C, "multiLvlStrRef")
        .and_then(|r| r.child(ns::C, "multiLvlStrCache"))
        .or_else(|| cat.child(ns::C, "multiLvlStrLit"))?;
    let count: usize = cache
        .child(ns::C, "ptCount")
        .and_then(|c| c.attr("val"))
        .and_then(|v| v.parse().ok())?;
    // Stored innermost first; we return outermost first.
    let lvls: Vec<&Element> = cache.children_named(ns::C, "lvl").collect();
    let mut out = vec![Vec::with_capacity(lvls.len()); count];
    for lvl in lvls.iter().rev() {
        let mut current = String::new();
        let mut labels: Vec<(usize, String)> = lvl
            .children_named(ns::C, "pt")
            .filter_map(|pt| {
                Some((
                    pt.attr("idx")?.parse().ok()?,
                    pt.child(ns::C, "v").map(Element::text).unwrap_or_default(),
                ))
            })
            .collect();
        labels.sort_by_key(|(i, _)| *i);
        let mut next = labels.into_iter().peekable();
        for (i, row) in out.iter_mut().enumerate() {
            while let Some((_, label)) = next.next_if(|(idx, _)| *idx <= i) {
                current = label;
            }
            row.push(current.clone());
        }
    }
    Some(out)
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

/// Sheet reference for the formulas we write (quoted if needed), based on
/// the sheet the template's formulas use.
///
/// Templates sometimes refer to linked workbooks (`[1]Data`) or use names
/// Excel would not accept; we write our own embedded workbook, so the name
/// is cleaned up to something valid.
fn sheet_ref(plot_area: &Element) -> String {
    let mut found = None;
    plot_area.walk(&mut |e| {
        if found.is_none()
            && e.is(ns::C, "f")
            && let Some((sheet, _)) = e.text().split_once('!')
        {
            found = Some(sheet.trim_start_matches('(').to_string());
        }
    });
    let name = clean_sheet_name(&unquote_sheet(&found.unwrap_or_default()));
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        name
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn clean_sheet_name(name: &str) -> String {
    // Drop an external workbook prefix such as "[1]".
    let name = match name.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((_, rest)) => rest,
        None => name,
    };
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .take(31)
        .collect();
    let cleaned = cleaned.trim().trim_matches('\'').to_string();
    if cleaned.is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned
    }
}

fn unquote_sheet(sheet: &str) -> String {
    sheet
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .map(|s| s.replace("''", "'"))
        .unwrap_or_else(|| sheet.to_string())
}

/// Plots in document order: their index among `plotArea`'s children and
/// the child indices of their `c:ser` elements.
fn plot_layout(plot_area: &Element) -> Vec<(usize, Vec<usize>)> {
    plot_area
        .children
        .iter()
        .enumerate()
        .filter_map(|(pi, node)| match node {
            Node::Element(plot) if is_plot(plot) => Some((
                pi,
                plot.children
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(ns::C, "ser")))
                    .map(|(si, _)| si)
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// Removes filtered (hidden) series. They live in extension lists and refer
/// to cells of the old workbook, which we replace, so after a data change
/// they would show stale data when someone unhides them.
fn drop_filtered_series(plot_area: &mut Element) {
    let is_filtered =
        |e: &Element| e.local().starts_with("filtered") && e.local().ends_with("Series");
    for plot in plot_area.elements_mut().filter(|e| is_plot(e)) {
        let Some(ext_lst) = plot.child_mut(ns::C, "extLst") else {
            continue;
        };
        ext_lst.remove_descendants(&is_filtered);
        ext_lst.remove_descendants(&|e| e.is(ns::C, "ext") && e.elements().next().is_none());
        if ext_lst.elements().next().is_none() {
            plot.remove_children(ns::C, "extLst");
        }
    }
}

/// Decides which plot (index into the layout) each requested series goes to.
fn assign_plots(
    layout: &[(usize, Vec<usize>)],
    types: &[String],
    requested: &[Option<usize>],
) -> Result<Vec<usize>> {
    let with_series: Vec<usize> = (0..layout.len())
        .filter(|&p| !layout[p].1.is_empty())
        .collect();
    if with_series.is_empty() {
        return Err(Error::Unsupported(
            "chart has no series to copy formatting from; add at least one in the template".into(),
        ));
    }
    let explicit = requested.iter().filter(|p| p.is_some()).count();
    if explicit == 0 {
        if with_series.len() == 1 {
            return Ok(vec![with_series[0]; requested.len()]);
        }
        let existing: Vec<usize> = layout
            .iter()
            .enumerate()
            .flat_map(|(p, (_, sers))| std::iter::repeat_n(p, sers.len()))
            .collect();
        if existing.len() == requested.len() {
            return Ok(existing);
        }
        return Err(Error::Unsupported(format!(
            "this is a combo chart ({}) with {} series; to pass {} series, say which plot \
             each one belongs to (Series.plot, an index into the chart types)",
            types.join(", "),
            existing.len(),
            requested.len()
        )));
    }
    if explicit != requested.len() {
        return Err(Error::InvalidArgument(
            "set the plot of every series or of none".into(),
        ));
    }
    let assignment: Vec<usize> = requested.iter().map(|p| p.expect("checked")).collect();
    for &p in &assignment {
        match layout.get(p) {
            None => {
                return Err(Error::InvalidArgument(format!(
                    "plot {p} does not exist; the chart has {} ({})",
                    layout.len(),
                    types.join(", ")
                )));
            }
            Some((_, sers)) if sers.is_empty() => {
                return Err(Error::Unsupported(format!(
                    "plot {p} ({}) has no series in the template to copy formatting from",
                    types[p]
                )));
            }
            _ => {}
        }
    }
    for &p in &with_series {
        if !assignment.contains(&p) {
            return Err(Error::Unsupported(format!(
                "plot {p} ({}) would be left without series; give it at least one",
                types[p]
            )));
        }
    }
    Ok(assignment)
}

/// Makes each plot hold as many series as are assigned to it (copying the
/// plot's last series, or removing from the end) and returns, for each
/// requested series in order, the location of its `c:ser` element.
fn arrange_series(plot_area: &mut Element, assignment: &[usize]) -> Vec<(usize, usize)> {
    let mut used_idx: Vec<u64> = Vec::new();
    let mut used_guids: HashSet<String> = HashSet::new();
    plot_area.walk(&mut |e| {
        if e.local() == "ser"
            && let Some(v) = e
                .child(ns::C, "idx")
                .and_then(|c| c.attr("val"))
                .and_then(|v| v.parse::<u64>().ok())
        {
            used_idx.push(v);
        }
        if e.local() == "uniqueId"
            && let Some(v) = e.attr("val")
        {
            used_guids.insert(v.to_string());
        }
    });

    let layout = plot_layout(plot_area);
    for (p, (pi, sers)) in layout.iter().enumerate() {
        let wanted = assignment.iter().filter(|&&a| a == p).count();
        let plot = child_el_mut(plot_area, *pi);
        if let Some(&last) = sers.last() {
            let template = child_el(plot, last).clone();
            for insert_at in (last + 1..).take(wanted.saturating_sub(sers.len())) {
                let mut copy = template.clone();
                let idx = used_idx.iter().max().map_or(0, |m| m + 1);
                used_idx.push(idx);
                if let Some(e) = copy.child_mut(ns::C, "idx") {
                    e.set_attr("val", idx.to_string());
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
        }
        for &si in sers.iter().skip(wanted).rev() {
            plot.children.remove(si);
        }
    }

    let layout = plot_layout(plot_area);
    let mut next = vec![0usize; layout.len()];
    let mut locs = Vec::with_capacity(assignment.len());
    for (k, &p) in assignment.iter().enumerate() {
        let (pi, sers) = &layout[p];
        let si = sers[next[p]];
        next[p] += 1;
        // Series are drawn and listed in c:order; follow the request order.
        if let Some(order) =
            child_el_mut(child_el_mut(plot_area, *pi), si).child_mut(ns::C, "order")
        {
            order.set_attr("val", k.to_string());
        }
        locs.push((*pi, si));
    }
    locs
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

pub(crate) fn read_categories(cat: &Element) -> Categories {
    if let Some(levels) = read_levels(cat) {
        return Categories::Levels(levels);
    }
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

/// Workbook columns holding the categories (one per level).
fn category_columns(categories: &Categories) -> Vec<Column> {
    let column = |cells| Column {
        header: None,
        cells,
        format: None,
    };
    match categories {
        Categories::Labels(v) => vec![column(
            v.iter().map(|s| Some(Cell::Text(s.clone()))).collect(),
        )],
        Categories::Numbers(v) | Categories::Dates(v) => {
            vec![column(v.iter().map(|x| Some(Cell::Number(*x))).collect())]
        }
        Categories::Levels(levels) => {
            let depth = categories.depth();
            (0..depth)
                .map(|level| {
                    // Like Excel's layout: a label only where its group starts.
                    let mut cells: Vec<Option<Cell>> = (0..levels.len()).map(|_| None).collect();
                    for (i, label) in group_starts(levels, level) {
                        cells[i] = Some(Cell::Text(label.to_string()));
                    }
                    column(cells)
                })
                .collect()
        }
    }
}

fn check_finite(what: &str, values: impl IntoIterator<Item = Option<f64>>) -> Result<()> {
    match values.into_iter().flatten().find(|v| !v.is_finite()) {
        Some(bad) => Err(Error::InvalidArgument(format!(
            "{what:?} contains {bad}; use None for missing values"
        ))),
        None => Ok(()),
    }
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
        let layout = plot_layout(pa);
        let mut series: Vec<(usize, &Element)> = layout
            .iter()
            .enumerate()
            .flat_map(|(p, (pi, sers))| {
                let plot = child_el(pa, *pi);
                sers.iter().map(move |&si| (p, child_el(plot, si)))
            })
            .collect();
        // Categories come from the first series in the file; series in a
        // damaged chart may disagree on them.
        let categories = match series.first().and_then(|(_, s)| s.child(ns::C, "cat")) {
            Some(cat) => read_categories(cat),
            None => Categories::Labels(Vec::new()),
        };
        // Present series in the order PowerPoint lists them.
        series.sort_by_key(|(_, ser)| {
            ser.child(ns::C, "order")
                .and_then(|o| o.attr("val"))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        });
        let combo = layout.iter().filter(|(_, sers)| !sers.is_empty()).count() > 1;
        Ok(ChartData {
            categories,
            series: series
                .into_iter()
                .map(|(p, ser)| {
                    let val = ser.child(ns::C, "val");
                    Series {
                        name: series_name(ser),
                        values: read_numbers(val),
                        plot: combo.then_some(p),
                        number_format: val.and_then(|v| read_points(v).1),
                    }
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
    /// Each plot keeps as many series as are assigned to it: extra series
    /// are copies of the plot's last series (formatting included), surplus
    /// ones are removed from the end. In combo charts, use [`Series::plot`]
    /// to say which plot a series belongs to; without it, the number of
    /// series must match the template.
    pub fn set_chart_data(&mut self, shape: ShapeRef, data: &ChartData) -> Result<()> {
        let n = data.categories.len();
        match &data.categories {
            Categories::Numbers(v) | Categories::Dates(v) => {
                check_finite("categories", v.iter().copied().map(Some))?;
            }
            Categories::Levels(v) => {
                let depth = data.categories.depth();
                if depth < 2 || v.iter().any(|row| row.len() != depth) {
                    return Err(Error::InvalidArgument(
                        "multi-level categories need at least two levels, the same for every \
                         category"
                            .into(),
                    ));
                }
            }
            Categories::Labels(_) => {}
        }
        for s in &data.series {
            check_finite(&s.name, s.values.iter().copied())?;
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
        let pa = plot_area(doc)?;
        if pa.elements().any(is_xy_plot) {
            return Err(Error::Unsupported(
                "this is a scatter or bubble chart; use set_chart_xy_data".into(),
            ));
        }
        let types: Vec<String> = plot_layout(pa)
            .iter()
            .map(|(pi, _)| child_el(pa, *pi).local().to_string())
            .collect();
        let requested: Vec<Option<usize>> = data.series.iter().map(|s| s.plot).collect();
        let assignment = assign_plots(&plot_layout(pa), &types, &requested)?;
        // Fail before touching the chart if the workbook can't be written.
        build_workbook(&unquote_sheet(&sheet_ref(pa)), &[])?;

        let doc = self.pkg.xml_mut(&part)?;
        let pa = plot_area_mut(doc)?;
        let sheet = sheet_ref(pa);
        drop_filtered_series(pa);
        let locs = arrange_series(pa, &assignment);

        let depth = data.categories.depth();
        let mut columns = category_columns(&data.categories);
        let cat_range = if depth > 1 {
            format!(
                "{sheet}!${}$2:${}${}",
                col_letter(0),
                col_letter(depth - 1),
                n.max(1) + 1
            )
        } else {
            range_ref(&sheet, 0, 1, n)
        };
        for (k, ((pi, si), series)) in locs.iter().zip(&data.series).enumerate() {
            let col = depth + k;
            let ser = child_el_mut(child_el_mut(pa, *pi), *si);
            write_name(ser, &series.name, &cell_ref(&sheet, col, 0));

            let cat = ser.ensure_child("cat", BEFORE_CAT);
            match &data.categories {
                Categories::Labels(v) => {
                    let labels: Vec<Option<String>> = v.iter().cloned().map(Some).collect();
                    write_data(
                        cat,
                        false,
                        &labels,
                        &cat_range,
                        NumberFormat::TemplateOr("General"),
                    );
                }
                Categories::Numbers(v) | Categories::Dates(v) => {
                    let fallback = if matches!(data.categories, Categories::Dates(_)) {
                        DATE_FORMAT
                    } else {
                        "General"
                    };
                    let strings: Vec<Option<String>> =
                        v.iter().map(|x| Some(format_number(*x))).collect();
                    let written = write_data(
                        cat,
                        true,
                        &strings,
                        &cat_range,
                        NumberFormat::TemplateOr(fallback),
                    );
                    if k == 0 {
                        columns[0].format = written;
                    }
                }
                Categories::Levels(levels) => write_levels(cat, levels, &cat_range),
            }

            let mut values = series.values.clone();
            values.resize(n, None);
            let val = ser.ensure_child("val", &before(&["cat"]));
            let format = match &series.number_format {
                Some(f) => NumberFormat::Explicit(f),
                None => NumberFormat::TemplateOr("General"),
            };
            let written = write_data(
                val,
                true,
                &number_strings(&values),
                &range_ref(&sheet, col, 1, n),
                format,
            );
            prune_points(ser, n);
            columns.push(Column {
                header: Some(series.name.clone()),
                cells: number_cells(&values),
                format: written,
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
        build_workbook(&unquote_sheet(&sheet_ref(pa)), &[])?;
        for s in series {
            check_finite(&s.name, s.x.iter().copied())?;
            check_finite(&s.name, s.y.iter().copied())?;
            check_finite(&s.name, s.sizes.iter().flatten().copied())?;
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
        let types: Vec<String> = plot_layout(pa)
            .iter()
            .map(|(pi, _)| child_el(pa, *pi).local().to_string())
            .collect();
        let assignment = assign_plots(&plot_layout(pa), &types, &vec![None; series.len()])?;

        let doc = self.pkg.xml_mut(&part)?;
        let pa = plot_area_mut(doc)?;
        let sheet = sheet_ref(pa);
        drop_filtered_series(pa);
        let locs = arrange_series(pa, &assignment);
        let width = if bubble { 3 } else { 2 };
        let mut columns = Vec::new();
        for (k, ((pi, si), s)) in locs.iter().zip(series).enumerate() {
            let (xc, yc, bc) = (k * width, k * width + 1, k * width + 2);
            let n = s.x.len();
            let ser = child_el_mut(child_el_mut(pa, *pi), *si);
            write_name(ser, &s.name, &cell_ref(&sheet, yc, 0));
            let template = NumberFormat::TemplateOr("General");
            let x = ser.ensure_child("xVal", BEFORE_CAT);
            let x_format = write_data(
                x,
                true,
                &number_strings(&s.x),
                &range_ref(&sheet, xc, 1, n),
                NumberFormat::TemplateOr("General"),
            );
            let y = ser.ensure_child("yVal", &before(&["xVal"]));
            let y_format = write_data(
                y,
                true,
                &number_strings(&s.y),
                &range_ref(&sheet, yc, 1, n),
                template,
            );
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
                let b_format = write_data(
                    b,
                    true,
                    &number_strings(sizes),
                    &range_ref(&sheet, bc, 1, n),
                    NumberFormat::TemplateOr("General"),
                );
                columns.push(Column {
                    header: Some("Size".into()),
                    cells: number_cells(sizes),
                    format: b_format,
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

    fn levels(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn group_starts_by_level() {
        let l = levels(&[&["A", "x"], &["A", "y"], &["B", "x"], &["A", "x"]]);
        assert_eq!(group_starts(&l, 0), [(0, "A"), (2, "B"), (3, "A")]);
        assert_eq!(
            group_starts(&l, 1),
            [(0, "x"), (1, "y"), (2, "x"), (3, "x")]
        );
    }

    #[test]
    fn multi_level_round_trip() {
        let l = levels(&[
            &["2025", "H1", "Jan"],
            &["2025", "H1", "Feb"],
            &["2025", "H2", "Jul"],
            &["2026", "H1", "Jan"],
        ]);
        let xml = r#"<c:cat xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let mut doc = Document::parse(xml.as_bytes()).unwrap();
        write_levels(&mut doc.root, &l, "Sheet1!$A$2:$C$5");
        assert_eq!(read_levels(&doc.root).unwrap(), l);
        let out = String::from_utf8(doc.to_bytes()).unwrap();
        // Innermost level first, outer levels only where groups start.
        let lvls: Vec<usize> = out
            .split("<c:lvl>")
            .skip(1)
            .map(|l| l.matches("<c:pt ").count())
            .collect();
        assert_eq!(lvls, [4, 3, 2]);
    }

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
        assert_eq!(clean_sheet_name("[0]Table 1"), "Table 1");
        assert_eq!(clean_sheet_name("a/b:c"), "abc");
        assert_eq!(clean_sheet_name(""), "Sheet1");
        assert_eq!(clean_sheet_name(&"x".repeat(40)).len(), 31);
    }

    #[test]
    fn numbers() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(0.1 + 0.2), "0.30000000000000004");
    }
}
