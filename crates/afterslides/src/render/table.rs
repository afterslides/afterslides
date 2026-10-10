//! Tables: grid, merged cells, cell fills and borders, table styles.

use kurbo::{Affine, BezPath, Rect, Shape};

use super::color::{ColorContext, child_color};
use super::display::{Item, LineCap, LineJoin, Paint, Rgba, Stroke};
use super::fill::{Fill, find_fill, resolve_fill, resolve_line};
use super::scene::{EMU_PER_PT, Layer, Scene};
use super::text::TextSources;
use crate::xml::{Element, ns};

/// The default table style PowerPoint applies and does not store in the
/// file ("Medium Style 2 - Accent 1"), and our fallback for any other
/// built-in style.
const MEDIUM_STYLE_2: &str = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}";

/// What a table style part contributes to one cell.
#[derive(Default, Clone)]
struct CellStyle {
    fill: Option<Paint>,
    bold: Option<bool>,
    color: Option<Rgba>,
    /// Borders: left, right, top, bottom, inside horizontal, inside vertical.
    borders: [Option<Stroke>; 6],
}

fn emu(el: Option<&Element>, name: &str, default: f64) -> f64 {
    el.and_then(|e| e.attr(name))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default)
        / EMU_PER_PT
}

fn white_line(width: f64) -> Stroke {
    Stroke {
        paint: Paint::Solid(Rgba::WHITE),
        width,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        dash: Vec::new(),
    }
}

fn tinted(base: Rgba, amount: f64) -> Rgba {
    let m = |c: u8| (f64::from(c) + (255.0 - f64::from(c)) * amount).round() as u8;
    Rgba::rgb(m(base.r), m(base.g), m(base.b))
}

/// The built-in default style, approximated.
fn medium_style_2(part: &str, accent: Rgba) -> CellStyle {
    let inner = white_line(1.0);
    let borders = || {
        [
            Some(inner.clone()),
            Some(inner.clone()),
            Some(inner.clone()),
            Some(inner.clone()),
            Some(inner.clone()),
            Some(inner.clone()),
        ]
    };
    match part {
        "wholeTbl" => CellStyle {
            fill: Some(Paint::Solid(tinted(accent, 0.8))),
            color: Some(Rgba::BLACK),
            borders: borders(),
            ..CellStyle::default()
        },
        "band1H" | "band1V" => CellStyle {
            fill: Some(Paint::Solid(tinted(accent, 0.6))),
            ..CellStyle::default()
        },
        "firstRow" | "lastRow" | "firstCol" | "lastCol" => CellStyle {
            fill: (part == "firstRow").then_some(Paint::Solid(accent)),
            bold: Some(true),
            color: (part == "firstRow").then_some(Rgba::WHITE),
            borders: if part == "firstRow" {
                borders()
            } else {
                Default::default()
            },
        },
        _ => CellStyle::default(),
    }
}

fn read_style_part(el: &Element, ctx: &ColorContext<'_>, theme: &super::color::Theme) -> CellStyle {
    let mut style = CellStyle::default();
    if let Some(txt) = el.child(ns::A, "tcTxStyle") {
        style.bold = txt.attr("b").map(|v| v == "on");
        style.color = child_color(txt, ctx);
    }
    if let Some(tc) = el.child(ns::A, "tcStyle") {
        let bounds = Rect::new(0.0, 0.0, 1.0, 1.0);
        if let Some(fill) = tc.child(ns::A, "fill").and_then(find_fill) {
            if let Some(Fill::Paint(p)) = resolve_fill(fill, ctx, bounds) {
                style.fill = Some(p);
            }
        } else if let Some(r) = tc.child(ns::A, "fillRef") {
            let color = child_color(r, ctx);
            if let Some((s, c)) =
                super::fill::style_ref(r, &theme.fill_styles, &theme.bg_fill_styles, ctx)
                && let Some(Fill::Paint(p)) =
                    resolve_fill(s, &ctx.with_placeholder(c.or(color)), bounds)
            {
                style.fill = Some(p);
            }
        }
        if let Some(bdr) = tc.child(ns::A, "tcBdr") {
            for (i, name) in ["left", "right", "top", "bottom", "insideH", "insideV"]
                .iter()
                .enumerate()
            {
                if let Some(ln) = bdr.child(ns::A, name).and_then(|b| b.child(ns::A, "ln")) {
                    style.borders[i] = resolve_line(ln, ctx, bounds);
                }
            }
        }
    }
    style
}

fn overlay(base: &mut CellStyle, top: &CellStyle) {
    if top.fill.is_some() {
        base.fill.clone_from(&top.fill);
    }
    if top.bold.is_some() {
        base.bold = top.bold;
    }
    if top.color.is_some() {
        base.color = top.color;
    }
    for (b, t) in base.borders.iter_mut().zip(&top.borders) {
        if t.is_some() {
            b.clone_from(t);
        }
    }
}

struct Cell<'a> {
    el: &'a Element,
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
}

impl<'a> Scene<'a, '_> {
    pub(super) fn table(
        &mut self,
        tbl: &'a Element,
        _layer: Layer<'a>,
        bounds: Rect,
        transform: Affine,
    ) {
        // Owned copies, so the scene can be borrowed mutably for the text.
        let theme = self.layers.theme.clone();
        let color_map = self.layers.color_map.clone();
        let colors = ColorContext {
            theme: &theme,
            map: &color_map,
            placeholder: None,
        };
        let props = tbl.child(ns::A, "tblPr");
        let flag = |n: &str| {
            props
                .and_then(|p| p.attr(n))
                .is_some_and(|v| matches!(v, "1" | "true"))
        };

        let widths: Vec<f64> = tbl
            .child(ns::A, "tblGrid")
            .map(|g| {
                g.children_named(ns::A, "gridCol")
                    .map(|c| emu(Some(c), "w", 0.0))
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<&Element> = tbl.children_named(ns::A, "tr").collect();
        let mut heights: Vec<f64> = rows.iter().map(|r| emu(Some(r), "h", 0.0)).collect();
        if widths.is_empty() || rows.is_empty() {
            return;
        }

        // Cells that are not hidden by a merge.
        let mut cells = Vec::new();
        for (r, tr) in rows.iter().enumerate() {
            for (c, tc) in tr.children_named(ns::A, "tc").enumerate() {
                let merged = |n: &str| matches!(tc.attr(n), Some("1" | "true"));
                if merged("hMerge") || merged("vMerge") || c >= widths.len() {
                    continue;
                }
                let span = |n: &str| {
                    tc.attr(n)
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1)
                };
                cells.push(Cell {
                    el: tc,
                    row: r,
                    col: c,
                    rows: span("rowSpan").min(rows.len() - r),
                    cols: span("gridSpan").min(widths.len() - c),
                });
            }
        }

        // Table style.
        let style_id = props
            .and_then(|p| p.child(ns::A, "tableStyleId"))
            .map(Element::text)
            .unwrap_or_else(|| MEDIUM_STYLE_2.to_string());
        let stored = self.table_style(&style_id);
        let accent = theme
            .color("accent1")
            .unwrap_or(Rgba::rgb(0x44, 0x72, 0xC4));
        let part_style = |part: &str| -> CellStyle {
            match stored.and_then(|s| s.child(ns::A, part)) {
                Some(el) => read_style_part(el, &colors, &theme),
                None if stored.is_none() => medium_style_2(part, accent),
                None => CellStyle::default(),
            }
        };
        let (n_rows, n_cols) = (rows.len(), widths.len());
        let style_for = |cell: &Cell<'_>| {
            let mut s = part_style("wholeTbl");
            let data_row = cell.row.saturating_sub(usize::from(flag("firstRow")));
            if flag("bandRow") && !(flag("firstRow") && cell.row == 0) && data_row.is_multiple_of(2) {
                overlay(&mut s, &part_style("band1H"));
            }
            if flag("bandCol") && cell.col.is_multiple_of(2) {
                overlay(&mut s, &part_style("band1V"));
            }
            if flag("firstCol") && cell.col == 0 {
                overlay(&mut s, &part_style("firstCol"));
            }
            if flag("lastCol") && cell.col + cell.cols == n_cols {
                overlay(&mut s, &part_style("lastCol"));
            }
            if flag("lastRow") && cell.row + cell.rows == n_rows {
                overlay(&mut s, &part_style("lastRow"));
            }
            if flag("firstRow") && cell.row == 0 {
                overlay(&mut s, &part_style("firstRow"));
            }
            s
        };

        // Rows grow to fit their text: measure first (single-row cells).
        let x_at = |c: usize| widths[..c].iter().sum::<f64>();
        for cell in &cells {
            if cell.rows != 1 {
                continue;
            }
            let width = widths[cell.col..cell.col + cell.cols].iter().sum::<f64>();
            let style = style_for(cell);
            let saved = std::mem::take(&mut self.items);
            let needed = self.cell_text(
                cell.el,
                Rect::new(0.0, 0.0, width, 1e6),
                &style,
                Affine::IDENTITY,
            );
            self.items = saved;
            heights[cell.row] = heights[cell.row].max(needed);
        }
        let y_at = |r: usize, heights: &[f64]| heights[..r].iter().sum::<f64>();
        let _ = bounds;

        // Fills, then text, then borders on top.
        let mut borders: Vec<(BezPath, Stroke)> = Vec::new();
        for cell in &cells {
            let rect = Rect::new(
                x_at(cell.col),
                y_at(cell.row, &heights),
                x_at(cell.col + cell.cols),
                y_at(cell.row + cell.rows, &heights),
            );
            let style = style_for(cell);
            let tc_pr = cell.el.child(ns::A, "tcPr");
            let own_fill = tc_pr
                .and_then(find_fill)
                .and_then(|f| resolve_fill(f, &colors, rect));
            let fill = match own_fill {
                Some(Fill::Paint(p)) => Some(p),
                Some(_) => None,
                None => style.fill.clone(),
            };
            if let Some(paint) = fill {
                self.items.push(Item::Fill {
                    path: rect.to_path(0.1),
                    paint,
                    transform,
                    even_odd: false,
                });
            }
            self.cell_text(cell.el, rect, &style, transform);

            // Borders: explicit ones on the cell, else the style's.
            let edges = [
                (
                    "lnL",
                    0,
                    (rect.x0, rect.y0, rect.x0, rect.y1),
                    cell.col == 0,
                ),
                (
                    "lnR",
                    1,
                    (rect.x1, rect.y0, rect.x1, rect.y1),
                    cell.col + cell.cols == n_cols,
                ),
                (
                    "lnT",
                    2,
                    (rect.x0, rect.y0, rect.x1, rect.y0),
                    cell.row == 0,
                ),
                (
                    "lnB",
                    3,
                    (rect.x0, rect.y1, rect.x1, rect.y1),
                    cell.row + cell.rows == n_rows,
                ),
            ];
            for (name, idx, (x0, y0, x1, y1), outer) in edges {
                let explicit = tc_pr.and_then(|p| p.child(ns::A, name));
                let stroke = match explicit {
                    Some(ln) => resolve_line(ln, &colors, rect),
                    None => {
                        let inside = if idx < 2 { 5 } else { 4 };
                        style.borders[if outer { idx } else { inside }].clone()
                    }
                };
                if let Some(stroke) = stroke {
                    let mut path = BezPath::new();
                    path.move_to((x0, y0));
                    path.line_to((x1, y1));
                    borders.push((path, stroke));
                }
            }
        }
        for (path, stroke) in borders {
            self.items.push(Item::Stroke {
                path,
                stroke,
                transform,
            });
        }
    }

    fn table_style(&self, id: &str) -> Option<&'a Element> {
        let prs = self.layers.prs;
        let part = prs
            .pkg
            .targets(&prs.main)
            .into_iter()
            .find(|(r, _)| r.rel_type.ends_with("/tableStyles"))
            .map(|(_, t)| t)?;
        let root = &prs.pkg.xml(&part).ok()?.root;
        root.children_named(ns::A, "tblStyle")
            .find(|s| s.attr("styleId") == Some(id))
    }

    /// Draws (or, with a scratch item list, measures) a cell's text.
    fn cell_text(
        &mut self,
        tc: &'a Element,
        rect: Rect,
        style: &CellStyle,
        transform: Affine,
    ) -> f64 {
        let Some(body) = tc.child(ns::A, "txBody") else {
            return 0.0;
        };
        let tc_pr = tc.child(ns::A, "tcPr");
        let prs_root = self
            .layers
            .prs
            .pkg
            .xml(&self.layers.prs.main)
            .ok()
            .map(|d| &d.root);
        let master_styles = prs_root
            .and_then(|r| r.child(ns::P, "defaultTextStyle"))
            .into_iter()
            .collect();
        let sources = TextSources {
            master_styles,
            font_ref: None,
            shape_styles: body.child(ns::A, "lstStyle").into_iter().collect(),
            body_props: body.child(ns::A, "bodyPr").into_iter().collect(),
            insets: Some([
                emu(tc_pr, "marL", 91440.0),
                emu(tc_pr, "marT", 45720.0),
                emu(tc_pr, "marR", 91440.0),
                emu(tc_pr, "marB", 45720.0),
            ]),
            anchor: tc_pr.and_then(|p| p.attr("anchor")).map(str::to_string),
            style_run: Some((style.bold, style.color)),
        };
        self.draw_text_body(body, &sources, rect, transform)
    }
}
