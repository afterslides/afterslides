//! Charts: column/bar, line, area and pie/doughnut, drawn from the value
//! caches in the chart part. Layout follows PowerPoint's automatic layout
//! loosely; manual layouts (`c:manualLayout`) are not applied yet.

use std::f64::consts::PI;

use kurbo::{Affine, BezPath, Circle, Point, Rect, Shape};

use super::color::{ColorContext, child_color};
use super::display::{Item, LineCap, LineJoin, Paint, Rgba, Stroke};
use super::fill::{Fill, find_fill, resolve_fill};
use super::scene::{Layer, Scene};
use super::text::{HAlign, Label};
use crate::Categories;
use crate::chart::{read_categories, read_numbers, read_points, series_name};
use crate::xml::{Element, ns};

const TEXT_GRAY: Rgba = Rgba::rgb(0x59, 0x59, 0x59);
const GRID_GRAY: Rgba = Rgba::rgb(0xD9, 0xD9, 0xD9);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
}

struct SeriesData<'a> {
    el: &'a Element,
    name: String,
    values: Vec<Option<f64>>,
    color: Rgba,
}

struct Plot<'a> {
    kind: Kind,
    el: &'a Element,
    series: Vec<SeriesData<'a>>,
}

fn line(color: Rgba, width: f64) -> Stroke {
    Stroke {
        paint: Paint::Solid(color),
        width,
        cap: LineCap::Butt,
        join: LineJoin::Round,
        dash: Vec::new(),
    }
}

/// Default series colours: the six accents, then darker and lighter
/// variants, as PowerPoint cycles them.
fn palette(ctx: &ColorContext<'_>, i: usize) -> Rgba {
    let base = ctx
        .theme
        .color(&format!("accent{}", i % 6 + 1))
        .unwrap_or(Rgba::rgb(0x44, 0x72, 0xC4));
    let variant = i / 6;
    let f = match variant {
        0 => return base,
        1 => 0.6,
        2 => 1.4,
        _ => 0.8,
    };
    let m = |c: u8| (f64::from(c) * f).clamp(0.0, 255.0).round() as u8;
    Rgba::rgb(m(base.r), m(base.g), m(base.b))
}

fn series_color(ser: &Element, ctx: &ColorContext<'_>, kind: Kind, i: usize) -> Rgba {
    let sp = ser.child(ns::C, "spPr");
    let from_fill = sp
        .and_then(|s| s.child(ns::A, "solidFill"))
        .and_then(|f| child_color(f, ctx));
    let from_line = sp
        .and_then(|s| s.path(&[(ns::A, "ln"), (ns::A, "solidFill")]))
        .and_then(|f| child_color(f, ctx));
    let explicit = if kind == Kind::Line {
        from_line.or(from_fill)
    } else {
        from_fill.or(from_line)
    };
    explicit.unwrap_or_else(|| palette(ctx, i))
}

fn gray_label(text: &str, size: f64) -> Label<'_> {
    Label {
        text,
        size,
        bold: false,
        color: TEXT_GRAY,
        font: None,
    }
}

/// Formats a number with a (simplified) Excel number format.
pub(super) fn format_value(v: f64, code: &str) -> String {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        let rounded = (v * 1e9).round() / 1e9;
        return if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
            format!("{}", rounded as i64)
        } else {
            format!("{rounded}")
        };
    }
    let section = code.split(';').next().unwrap_or(code);
    let percent = section.contains('%');
    let value = if percent { v * 100.0 } else { v };
    let decimals = section.split_once('.').map_or(0, |(_, rest)| {
        rest.chars().take_while(|c| matches!(c, '0' | '#')).count()
    });
    let mut out = format!("{:.*}", decimals, value.abs());
    if section.contains(',') {
        let (int, frac) = out
            .split_once('.')
            .map_or((out.as_str(), None), |(a, b)| (a, Some(b)));
        let mut grouped = String::new();
        for (i, ch) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        out = match frac {
            Some(f) => format!("{grouped}.{f}"),
            None => grouped,
        };
    }
    if value < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    if percent {
        out.push('%');
    }
    out
}

/// Axis range and major unit for values between `lo` and `hi`, the way
/// PowerPoint picks them: about 5% headroom beyond the data, then rounded
/// out to a "nice" major unit.
fn nice_scale(lo: f64, hi: f64) -> (f64, f64, f64) {
    let (lo, hi) = (lo.min(0.0), hi.max(0.0));
    let range = (hi - lo).max(f64::EPSILON);
    let hi = if hi > 0.0 { hi + range * 0.05 } else { hi };
    let lo = if lo < 0.0 { lo - range * 0.05 } else { lo };
    let raw = (hi - lo).max(f64::EPSILON) / 8.0;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw * 0.999)
        .unwrap_or(10.0 * mag);
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

impl<'a> Scene<'a, '_> {
    pub(super) fn chart_frame(
        &mut self,
        frame: &'a Element,
        layer: Layer<'a>,
        bounds: Rect,
        transform: Affine,
    ) {
        let Some(chart_space) = self.chart_space(frame, layer) else {
            self.unsupported_box(bounds, transform);
            return;
        };
        // Owned copies, so labels (which borrow the scene) can be drawn.
        let theme = self.layers.theme.clone();
        let color_map = self.layers.color_map.clone();
        let ctx = ColorContext {
            theme: &theme,
            map: &color_map,
            placeholder: None,
        };
        let Some(chart) = chart_space.child(ns::C, "chart") else {
            return;
        };
        let Some(plot_area) = chart.child(ns::C, "plotArea") else {
            return;
        };

        let mut plots = Vec::new();
        for el in plot_area.elements() {
            let kind = match el.local() {
                "barChart" | "bar3DChart" => Kind::Bar,
                "lineChart" | "line3DChart" | "stockChart" => Kind::Line,
                "areaChart" | "area3DChart" => Kind::Area,
                "pieChart" | "pie3DChart" | "ofPieChart" => Kind::Pie,
                "doughnutChart" => Kind::Doughnut,
                _ => continue,
            };
            let mut series = Vec::new();
            for ser in el.children_named(ns::C, "ser") {
                let idx = ser
                    .child(ns::C, "idx")
                    .and_then(|i| i.attr("val"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(series.len());
                series.push(SeriesData {
                    el: ser,
                    name: series_name(ser),
                    values: read_numbers(ser.child(ns::C, "val")),
                    color: series_color(ser, &ctx, kind, idx),
                });
            }
            plots.push(Plot { kind, el, series });
        }
        if plots.is_empty() || plots.iter().all(|p| p.series.is_empty()) {
            self.unsupported_box(bounds, transform);
            return;
        }

        // Chart area.
        if let Some(fill) = chart_space.child(ns::C, "spPr").and_then(find_fill)
            && let Some(Fill::Paint(paint)) = resolve_fill(fill, &ctx, bounds)
        {
            self.items.push(Item::Fill {
                path: bounds.to_path(0.1),
                paint,
                transform,
                even_odd: false,
            });
        }

        let base_size = chart_space
            .path(&[
                (ns::C, "txPr"),
                (ns::A, "p"),
                (ns::A, "pPr"),
                (ns::A, "defRPr"),
            ])
            .and_then(|r| r.attr("sz"))
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(10.0, |v| v / 100.0);
        let pad = 7.0;
        let mut area = bounds.inset(-pad);

        // Title.
        let deleted = chart
            .child(ns::C, "autoTitleDeleted")
            .is_some_and(|d| matches!(d.attr("val"), Some("1" | "true")));
        // Charts with one series get an automatic title (the series name)
        // unless the title was deleted.
        let single = plots.iter().map(|p| p.series.len()).sum::<usize>() == 1;
        let auto_title = || {
            single
                .then(|| {
                    plots
                        .iter()
                        .flat_map(|p| p.series.first())
                        .next()
                        .map(|s| s.name.clone())
                })
                .flatten()
        };
        let title_el = chart.child(ns::C, "title");
        if !deleted && (title_el.is_some() || single) {
            let text = title_el
                .and_then(|t| t.path(&[(ns::C, "tx"), (ns::C, "rich")]))
                .map(crate::text::get_text)
                .or_else(|| {
                    title_el
                        .and_then(|t| t.child(ns::C, "tx"))
                        .and_then(|t| read_points(t).0.into_iter().next().flatten())
                })
                .or_else(auto_title)
                .unwrap_or_default();
            if !text.is_empty() {
                let size = title_el
                    .and_then(|t| t.path(&[(ns::C, "tx"), (ns::C, "rich")]))
                    .and_then(|r| {
                        let mut sz = None;
                        r.walk(&mut |e| {
                            if sz.is_none() && (e.is(ns::A, "rPr") || e.is(ns::A, "defRPr")) {
                                sz = e.attr("sz").and_then(|v| v.parse::<f64>().ok());
                            }
                        });
                        sz
                    })
                    .map_or(base_size * 1.4, |v| v / 100.0);
                let label = Label {
                    text: &text,
                    size,
                    bold: true,
                    color: TEXT_GRAY,
                    font: None,
                };
                let (_, h) =
                    self.draw_label(&label, area.center().x, area.y0, HAlign::Center, transform);
                area.y0 += h + 4.0;
            }
        }

        // Legend entries: series, or categories for pie-like charts.
        let pie_like = plots[0].kind == Kind::Pie || plots[0].kind == Kind::Doughnut;
        let first_series = plots.iter().flat_map(|p| p.series.first()).next();
        let categories: Vec<String> = first_series
            .and_then(|s| s.el.child(ns::C, "cat"))
            .map(|c| match read_categories(c) {
                Categories::Labels(v) => v,
                Categories::Numbers(v) | Categories::Dates(v) => {
                    let fmt = read_points(c).1.unwrap_or_default();
                    v.iter().map(|x| format_value(*x, &fmt)).collect()
                }
                Categories::Levels(v) => v
                    .into_iter()
                    .map(|l| l.last().cloned().unwrap_or_default())
                    .collect(),
            })
            .unwrap_or_default();
        let vary = plots[0]
            .el
            .child(ns::C, "varyColors")
            .is_some_and(|v| matches!(v.attr("val"), Some("1" | "true") | None));
        let point_colors: Vec<Rgba> = (0..categories.len().max(1))
            .map(|i| {
                first_series
                    .and_then(|s| {
                        s.el.children_named(ns::C, "dPt").find(|d| {
                            d.child(ns::C, "idx").and_then(|x| x.attr("val"))
                                == Some(&i.to_string())
                        })
                    })
                    .and_then(|d| d.path(&[(ns::C, "spPr"), (ns::A, "solidFill")]))
                    .and_then(|f| child_color(f, &ctx))
                    .unwrap_or_else(|| palette(&ctx, i))
            })
            .collect();

        if let Some(legend) = chart.child(ns::C, "legend") {
            let pos = legend
                .child(ns::C, "legendPos")
                .and_then(|p| p.attr("val"))
                .unwrap_or("r")
                .to_string();
            let entries: Vec<(String, Rgba)> = if pie_like && vary {
                categories
                    .iter()
                    .cloned()
                    .zip(point_colors.iter().copied())
                    .collect()
            } else {
                plots
                    .iter()
                    .flat_map(|p| p.series.iter().map(|s| (s.name.clone(), s.color)))
                    .collect()
            };
            area = self.legend(&entries, &pos, area, base_size, transform);
        }

        if pie_like {
            self.pie(
                &plots[0],
                &point_colors,
                plots[0].kind == Kind::Doughnut,
                area,
                transform,
            );
            return;
        }
        self.category_chart(&plots, plot_area, &categories, area, base_size, transform);
    }

    fn chart_space(&self, frame: &'a Element, layer: Layer<'a>) -> Option<&'a Element> {
        let rid = frame
            .path(&[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::C, "chart")])?
            .attr_ns(ns::R, "id")?;
        let part = self.layers.prs.pkg.resolve(layer.part, rid).ok()?;
        let name = self.layers.prs.pkg.part_names().find(|p| *p == part)?;
        Some(&self.layers.prs.pkg.xml(name).ok()?.root)
    }

    /// Draws the legend and returns the area left for the plot.
    fn legend(
        &mut self,
        entries: &[(String, Rgba)],
        pos: &str,
        area: Rect,
        size: f64,
        transform: Affine,
    ) -> Rect {
        let marker = size * 0.7;
        let gap = 6.0;
        let measured: Vec<(f64, f64)> = entries
            .iter()
            .map(|(name, _)| {
                self.measure_label(&Label {
                    text: name,
                    size,
                    bold: false,
                    color: TEXT_GRAY,
                    font: None,
                })
            })
            .collect();
        let line_h = measured.iter().map(|m| m.1).fold(size * 1.2, f64::max);
        let draw_entry = |scene: &mut Self, i: usize, x: f64, y: f64| {
            let (name, color) = &entries[i];
            let square = Rect::new(
                x,
                y + (line_h - marker) / 2.0,
                x + marker,
                y + (line_h + marker) / 2.0,
            );
            scene.items.push(Item::Fill {
                path: square.to_path(0.1),
                paint: Paint::Solid(*color),
                transform,
                even_odd: false,
            });
            scene.draw_label(
                &Label {
                    text: name,
                    size,
                    bold: false,
                    color: TEXT_GRAY,
                    font: None,
                },
                x + marker + 3.0,
                y,
                HAlign::Left,
                transform,
            );
        };
        match pos {
            "b" | "t" => {
                let total: f64 = measured
                    .iter()
                    .map(|m| marker + 3.0 + m.0 + gap * 2.0)
                    .sum::<f64>()
                    - gap * 2.0;
                let mut x = area.center().x - total / 2.0;
                let y = if pos == "b" {
                    area.y1 - line_h
                } else {
                    area.y0
                };
                for (i, m) in measured.iter().enumerate() {
                    draw_entry(self, i, x, y);
                    x += marker + 3.0 + m.0 + gap * 2.0;
                }
                if pos == "b" {
                    Rect::new(area.x0, area.y0, area.x1, area.y1 - line_h - gap)
                } else {
                    Rect::new(area.x0, area.y0 + line_h + gap, area.x1, area.y1)
                }
            }
            _ => {
                let width = measured.iter().map(|m| m.0).fold(0.0, f64::max) + marker + 3.0;
                let total = line_h * entries.len() as f64;
                let x = if pos == "l" { area.x0 } else { area.x1 - width };
                let mut y = area.center().y - total / 2.0;
                for i in 0..entries.len() {
                    draw_entry(self, i, x, y);
                    y += line_h;
                }
                if pos == "l" {
                    Rect::new(area.x0 + width + gap, area.y0, area.x1, area.y1)
                } else {
                    Rect::new(area.x0, area.y0, area.x1 - width - gap, area.y1)
                }
            }
        }
    }

    fn pie(
        &mut self,
        plot: &Plot<'_>,
        colors: &[Rgba],
        doughnut: bool,
        area: Rect,
        transform: Affine,
    ) {
        let Some(series) = plot.series.first() else {
            return;
        };
        let values: Vec<f64> = series
            .values
            .iter()
            .map(|v| v.unwrap_or(0.0).max(0.0))
            .collect();
        let total: f64 = values.iter().sum();
        if total <= 0.0 {
            return;
        }
        let center = area.center();
        let radius = area.width().min(area.height()) / 2.0 * 0.95;
        let hole = if doughnut {
            plot.el
                .child(ns::C, "holeSize")
                .and_then(|h| h.attr("val"))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(50.0)
                / 100.0
                * radius
        } else {
            0.0
        };
        let first = plot
            .el
            .child(ns::C, "firstSliceAng")
            .and_then(|a| a.attr("val"))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        // Slices start at 12 o'clock and run clockwise.
        let mut angle = (first - 90.0).to_radians();
        for (i, v) in values.iter().enumerate() {
            let sweep = v / total * 2.0 * PI;
            let mut path = BezPath::new();
            let arc = kurbo::Arc::new(center, (radius, radius), angle, sweep, 0.0);
            if hole > 0.0 {
                let inner = kurbo::Arc::new(center, (hole, hole), angle + sweep, -sweep, 0.0);
                path.move_to(center + kurbo::Vec2::from_angle(angle) * radius);
                arc.to_cubic_beziers(0.1, |a, b, p| path.curve_to(a, b, p));
                path.line_to(center + kurbo::Vec2::from_angle(angle + sweep) * hole);
                inner.to_cubic_beziers(0.1, |a, b, p| path.curve_to(a, b, p));
            } else {
                path.move_to(center);
                path.line_to(center + kurbo::Vec2::from_angle(angle) * radius);
                arc.to_cubic_beziers(0.1, |a, b, p| path.curve_to(a, b, p));
            }
            path.close_path();
            let color = colors.get(i).copied().unwrap_or(series.color);
            self.items.push(Item::Fill {
                path: path.clone(),
                paint: Paint::Solid(color),
                transform,
                even_odd: false,
            });
            self.items.push(Item::Stroke {
                path,
                stroke: line(Rgba::WHITE, 0.75),
                transform,
            });
            angle += sweep;
        }
    }

    fn category_chart(
        &mut self,
        plots: &[Plot<'_>],
        plot_area: &Element,
        categories: &[String],
        area: Rect,
        size: f64,
        transform: Affine,
    ) {
        let n_cat = categories
            .len()
            .max(
                plots
                    .iter()
                    .flat_map(|p| p.series.iter().map(|s| s.values.len()))
                    .max()
                    .unwrap_or(0),
            )
            .max(1);
        let horizontal = plots.iter().any(|p| {
            p.kind == Kind::Bar
                && p.el.child(ns::C, "barDir").and_then(|d| d.attr("val")) == Some("bar")
        });
        let grouping = |p: &Plot<'_>| {
            p.el.child(ns::C, "grouping")
                .and_then(|g| g.attr("val"))
                .unwrap_or("clustered")
                .to_string()
        };
        let stacked = |p: &Plot<'_>| matches!(grouping(p).as_str(), "stacked" | "percentStacked");
        let percent = |p: &Plot<'_>| grouping(p) == "percentStacked";

        // Value range over all plots, with stacking.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in plots {
            if stacked(p) {
                for c in 0..n_cat {
                    let (mut pos, mut neg) = (0.0, 0.0);
                    for s in &p.series {
                        let v = s.values.get(c).copied().flatten().unwrap_or(0.0);
                        if v >= 0.0 { pos += v } else { neg += v }
                    }
                    if percent(p) {
                        (pos, neg) = (
                            if pos > 0.0 { 1.0 } else { 0.0 },
                            if neg < 0.0 { -1.0 } else { 0.0 },
                        );
                    }
                    lo = lo.min(neg);
                    hi = hi.max(pos);
                }
            } else {
                for v in p.series.iter().flat_map(|s| s.values.iter().flatten()) {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
            }
        }
        if !lo.is_finite() {
            (lo, hi) = (0.0, 1.0);
        }
        let val_ax = plot_area.child(ns::C, "valAx");
        let scaling = val_ax.and_then(|a| a.child(ns::C, "scaling"));
        let fixed = |n: &str| {
            scaling
                .and_then(|s| s.child(ns::C, n))
                .and_then(|m| m.attr("val"))
                .and_then(|v| v.parse::<f64>().ok())
        };
        let (mut vmin, mut vmax, step) = nice_scale(lo, hi);
        if let Some(m) = fixed("min") {
            vmin = m;
        }
        if let Some(m) = fixed("max") {
            vmax = m;
        }
        let step = val_ax
            .and_then(|a| a.child(ns::C, "majorUnit"))
            .and_then(|m| m.attr("val"))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(step);
        let format = val_ax
            .and_then(|a| a.child(ns::C, "numFmt"))
            .filter(|f| f.attr("sourceLinked") != Some("1"))
            .and_then(|f| f.attr("formatCode"))
            .map(str::to_string)
            .or_else(|| {
                plots[0]
                    .series
                    .first()
                    .and_then(|s| s.el.child(ns::C, "val"))
                    .and_then(|v| read_points(v).1)
            })
            .unwrap_or_default();
        let deleted = |ax: Option<&Element>| {
            ax.and_then(|a| a.child(ns::C, "delete"))
                .is_some_and(|d| matches!(d.attr("val"), Some("1" | "true")))
        };
        let show_val_labels = !deleted(val_ax);
        let cat_ax = plot_area
            .child(ns::C, "catAx")
            .or_else(|| plot_area.child(ns::C, "dateAx"));
        let show_cat_labels = !deleted(cat_ax);

        // Tick values and their labels.
        let mut ticks = Vec::new();
        let mut t = vmin;
        let max_ticks = 50;
        while t <= vmax + step * 1e-6 && ticks.len() < max_ticks && step > 0.0 {
            ticks.push(t);
            t += step;
        }
        let tick_labels: Vec<String> = ticks.iter().map(|v| format_value(*v, &format)).collect();
        let label = |text| gray_label(text, size);
        let val_width = if show_val_labels {
            tick_labels
                .iter()
                .map(|l| self.measure_label(&label(l)).0)
                .fold(0.0, f64::max)
        } else {
            0.0
        };
        let line_h = size * 1.25;
        let cat_width = if horizontal && show_cat_labels {
            categories
                .iter()
                .map(|c| self.measure_label(&label(c)).0)
                .fold(0.0, f64::max)
        } else {
            0.0
        };

        let plot = if horizontal {
            Rect::new(
                area.x0 + cat_width + 4.0,
                area.y0,
                area.x1,
                area.y1 - if show_val_labels { line_h } else { 0.0 },
            )
        } else {
            Rect::new(
                area.x0 + val_width + 4.0,
                area.y0 + size / 2.0,
                area.x1,
                area.y1 - if show_cat_labels { line_h + 2.0 } else { 0.0 },
            )
        };
        if plot.width() <= 1.0 || plot.height() <= 1.0 {
            return;
        }
        let span = (vmax - vmin).max(1e-12);
        // Maps (category position 0..n, value) to the page.
        let map = |c: f64, v: f64| -> Point {
            let vf = (v - vmin) / span;
            if horizontal {
                Point::new(
                    plot.x0 + vf * plot.width(),
                    plot.y1 - c / n_cat as f64 * plot.height(),
                )
            } else {
                Point::new(
                    plot.x0 + c / n_cat as f64 * plot.width(),
                    plot.y1 - vf * plot.height(),
                )
            }
        };

        // Gridlines and value labels.
        let gridlines = val_ax
            .and_then(|a| a.child(ns::C, "majorGridlines"))
            .is_some();
        for (v, text) in ticks.iter().zip(&tick_labels) {
            let a = map(0.0, *v);
            let b = map(n_cat as f64, *v);
            if gridlines {
                let mut p = BezPath::new();
                p.move_to(a);
                p.line_to(b);
                self.items.push(Item::Stroke {
                    path: p,
                    stroke: line(GRID_GRAY, 0.75),
                    transform,
                });
            }
            if show_val_labels {
                if horizontal {
                    self.draw_label(&label(text), a.x, plot.y1 + 2.0, HAlign::Center, transform);
                } else {
                    self.draw_label(
                        &label(text),
                        plot.x0 - 4.0,
                        a.y - line_h / 2.0,
                        HAlign::Right,
                        transform,
                    );
                }
            }
        }
        // Category axis line and labels.
        {
            let mut p = BezPath::new();
            p.move_to(map(0.0, vmin.max(0.0).min(vmax)));
            p.line_to(map(n_cat as f64, vmin.max(0.0).min(vmax)));
            self.items.push(Item::Stroke {
                path: p,
                stroke: line(GRID_GRAY, 0.75),
                transform,
            });
        }
        if show_cat_labels {
            for (i, c) in categories.iter().enumerate() {
                let at = map(i as f64 + 0.5, vmin);
                if horizontal {
                    self.draw_label(
                        &label(c),
                        plot.x0 - 4.0,
                        at.y - line_h / 2.0,
                        HAlign::Right,
                        transform,
                    );
                } else {
                    self.draw_label(&label(c), at.x, plot.y1 + 2.0, HAlign::Center, transform);
                }
            }
        }

        // Series.
        for p in plots {
            match p.kind {
                Kind::Bar => self.bars(p, n_cat, stacked(p), percent(p), &map, transform),
                Kind::Line | Kind::Area => self.lines(p, n_cat, stacked(p), &map, transform),
                _ => {}
            }
        }
    }

    fn bars(
        &mut self,
        p: &Plot<'_>,
        n_cat: usize,
        stacked: bool,
        percent: bool,
        map: &dyn Fn(f64, f64) -> Point,
        transform: Affine,
    ) {
        let gap =
            p.el.child(ns::C, "gapWidth")
                .and_then(|g| g.attr("val"))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(150.0)
                / 100.0;
        let overlap =
            p.el.child(ns::C, "overlap")
                .and_then(|o| o.attr("val"))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(if stacked { 100.0 } else { 0.0 })
                / 100.0;
        let n = p.series.len().max(1) as f64;
        let slots = if stacked {
            1.0
        } else {
            n - (n - 1.0) * overlap
        };
        let bar = 1.0 / (slots + gap);
        let step = if stacked { 0.0 } else { bar * (1.0 - overlap) };
        let group = bar + step * (n - 1.0);
        for c in 0..n_cat {
            let totals: f64 = p
                .series
                .iter()
                .map(|s| s.values.get(c).copied().flatten().unwrap_or(0.0).abs())
                .sum();
            let (mut pos, mut neg) = (0.0, 0.0);
            for (si, s) in p.series.iter().enumerate() {
                let Some(mut v) = s.values.get(c).copied().flatten() else {
                    continue;
                };
                if percent && totals > 0.0 {
                    v /= totals;
                }
                let start = c as f64 + 0.5 - group / 2.0 + si as f64 * step;
                let (base, top) = if stacked {
                    if v >= 0.0 {
                        pos += v;
                        (pos - v, pos)
                    } else {
                        neg += v;
                        (neg - v, neg)
                    }
                } else {
                    (0.0, v)
                };
                let a = map(start, base);
                let b = map(start + bar, top);
                let rect = Rect::from_points(a, b);
                self.items.push(Item::Fill {
                    path: rect.to_path(0.1),
                    paint: Paint::Solid(s.color),
                    transform,
                    even_odd: false,
                });
            }
        }
    }

    fn lines(
        &mut self,
        p: &Plot<'_>,
        n_cat: usize,
        stacked: bool,
        map: &dyn Fn(f64, f64) -> Point,
        transform: Affine,
    ) {
        let mut cumulative = vec![0.0; n_cat];
        let plot_markers =
            p.el.child(ns::C, "marker")
                .is_none_or(|m| matches!(m.attr("val"), Some("1" | "true") | None));
        for s in &p.series {
            let points: Vec<Option<Point>> = (0..n_cat)
                .map(|c| {
                    s.values.get(c).copied().flatten().map(|v| {
                        let v = if stacked {
                            cumulative[c] += v;
                            cumulative[c]
                        } else {
                            v
                        };
                        map(c as f64 + 0.5, v)
                    })
                })
                .collect();
            if p.kind == Kind::Area {
                let present: Vec<Point> = points.iter().flatten().copied().collect();
                if let (Some(first), Some(last)) = (present.first(), present.last()) {
                    let mut path = BezPath::new();
                    let base_y = map(0.0, 0.0).y;
                    path.move_to((first.x, base_y));
                    for pt in &present {
                        path.line_to(*pt);
                    }
                    path.line_to((last.x, base_y));
                    path.close_path();
                    self.items.push(Item::Fill {
                        path,
                        paint: Paint::Solid(s.color),
                        transform,
                        even_odd: false,
                    });
                }
                continue;
            }
            let mut path = BezPath::new();
            let mut pen_down = false;
            for pt in &points {
                match pt {
                    Some(pt) if pen_down => path.line_to(*pt),
                    Some(pt) => {
                        path.move_to(*pt);
                        pen_down = true;
                    }
                    None => pen_down = false,
                }
            }
            let width =
                s.el.path(&[(ns::C, "spPr"), (ns::A, "ln")])
                    .and_then(|l| l.attr("w"))
                    .and_then(|w| w.parse::<f64>().ok())
                    .map_or(2.25, |w| w / 12700.0);
            let no_line =
                s.el.path(&[(ns::C, "spPr"), (ns::A, "ln"), (ns::A, "noFill")])
                    .is_some();
            if !no_line {
                let mut stroke = line(s.color, width);
                stroke.cap = LineCap::Round;
                self.items.push(Item::Stroke {
                    path,
                    stroke,
                    transform,
                });
            }
            let symbol =
                s.el.path(&[(ns::C, "marker"), (ns::C, "symbol")])
                    .and_then(|m| m.attr("val"));
            let show = match symbol {
                Some("none") => false,
                Some(_) => true,
                None => plot_markers,
            };
            if show {
                let r =
                    s.el.path(&[(ns::C, "marker"), (ns::C, "size")])
                        .and_then(|m| m.attr("val"))
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(5.0)
                        / 2.0;
                for pt in points.iter().flatten() {
                    let marker = Circle::new(*pt, r).to_path(0.05);
                    self.items.push(Item::Fill {
                        path: marker,
                        paint: Paint::Solid(s.color),
                        transform,
                        even_odd: false,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats() {
        assert_eq!(format_value(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(format_value(0.256, "0%"), "26%");
        assert_eq!(format_value(0.256, "0.0%"), "25.6%");
        assert_eq!(format_value(14.0, "General"), "14");
        assert_eq!(format_value(-3.0, "0.00"), "-3.00");
        assert_eq!(format_value(2.5, ""), "2.5");
    }

    #[test]
    fn scales() {
        // The fixture's charts: PowerPoint draws 0..18 by 2 and 0..140 by 20.
        assert_eq!(nice_scale(9.75, 15.25), (0.0, 18.0, 2.0));
        assert_eq!(nice_scale(90.0, 120.0), (0.0, 140.0, 20.0));
        let (lo, hi, _) = nice_scale(-5.0, 7.0);
        assert!(lo <= -5.0 && hi >= 7.0);
    }
}
