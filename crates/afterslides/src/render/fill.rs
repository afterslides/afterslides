//! Fills and outlines: `a:solidFill`, `a:gradFill`, `a:blipFill`, `a:ln`,
//! and theme style references (`p:style`, `p:bgRef`).

use kurbo::{Point, Rect};

use super::color::{ColorContext, child_color, parse_fraction};
use super::display::{GradientStop, LineCap, LineJoin, Paint, Rgba, Stroke};
use crate::xml::{Element, ns};

/// What a fill element resolves to.
#[derive(Debug, Clone)]
pub enum Fill {
    /// `a:noFill`.
    None,
    Paint(Paint),
    /// `a:blipFill`: an image from relationship `rid`, with a crop.
    Picture {
        rid: String,
        crop: [f64; 4],
    },
}

pub fn is_fill(e: &Element) -> bool {
    e.ns() == Some(ns::A)
        && matches!(
            e.local(),
            "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill"
        )
}

/// The first fill element among `parent`'s children.
pub fn find_fill(parent: &Element) -> Option<&Element> {
    parent.elements().find(|e| is_fill(e))
}

pub fn resolve_fill(fill: &Element, ctx: &ColorContext<'_>, bounds: Rect) -> Option<Fill> {
    match fill.local() {
        "noFill" => Some(Fill::None),
        "solidFill" => child_color(fill, ctx).map(|c| Fill::Paint(Paint::Solid(c))),
        "gradFill" => gradient(fill, ctx, bounds).map(Fill::Paint),
        "blipFill" => {
            let blip = fill.child(ns::A, "blip")?;
            let rid = blip.attr_ns(crate::xml::ns::R, "embed")?.to_string();
            Some(Fill::Picture {
                rid,
                crop: crop_of(fill),
            })
        }
        // Patterns are drawn with their foreground colour for now.
        "pattFill" => fill
            .child(ns::A, "fgClr")
            .and_then(|c| child_color(c, ctx))
            .map(|c| Fill::Paint(Paint::Solid(c))),
        _ => None,
    }
}

/// `a:srcRect` as fractions (l, t, r, b).
pub fn crop_of(blip_fill: &Element) -> [f64; 4] {
    let Some(rect) = blip_fill.child(ns::A, "srcRect") else {
        return [0.0; 4];
    };
    let v = |n| rect.attr(n).map_or(0.0, parse_fraction);
    [v("l"), v("t"), v("r"), v("b")]
}

fn gradient(fill: &Element, ctx: &ColorContext<'_>, bounds: Rect) -> Option<Paint> {
    let mut stops: Vec<GradientStop> = fill
        .child(ns::A, "gsLst")?
        .children_named(ns::A, "gs")
        .filter_map(|gs| {
            Some(GradientStop {
                offset: gs.attr("pos").map_or(0.0, parse_fraction) as f32,
                color: child_color(gs, ctx)?,
            })
        })
        .collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    if stops.is_empty() {
        return None;
    }
    if fill.child(ns::A, "path").is_some() {
        let center = bounds.center();
        let radius = (bounds.width().hypot(bounds.height())) / 2.0;
        return Some(Paint::Radial {
            center,
            radius,
            stops,
        });
    }
    // a:lin ang is clockwise from the x axis, in 60000ths of a degree.
    let angle = fill
        .child(ns::A, "lin")
        .and_then(|l| l.attr("ang"))
        .and_then(|a| a.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 60000.0;
    let (sin, cos) = angle.to_radians().sin_cos();
    let half = (bounds.width() * cos.abs() + bounds.height() * sin.abs()) / 2.0;
    let c = bounds.center();
    Some(Paint::Linear {
        start: Point::new(c.x - cos * half, c.y - sin * half),
        end: Point::new(c.x + cos * half, c.y + sin * half),
        stops,
    })
}

/// Resolves an `a:ln` into a stroke; `None` for no line.
pub fn resolve_line(ln: &Element, ctx: &ColorContext<'_>, bounds: Rect) -> Option<Stroke> {
    let paint = match resolve_fill(find_fill(ln)?, ctx, bounds)? {
        Fill::Paint(p) => p,
        Fill::None | Fill::Picture { .. } => return None,
    };
    // Width in EMU; 0 means hairline. Default is 9525 EMU (0.75 pt).
    let width = ln
        .attr("w")
        .and_then(|w| w.parse::<f64>().ok())
        .unwrap_or(9525.0)
        / 12700.0;
    let cap = match ln.attr("cap") {
        Some("rnd") => LineCap::Round,
        Some("sq") => LineCap::Square,
        _ => LineCap::Butt,
    };
    let join = if ln.child(ns::A, "round").is_some() {
        LineJoin::Round
    } else if ln.child(ns::A, "bevel").is_some() {
        LineJoin::Bevel
    } else {
        LineJoin::Miter
    };
    let w = width.max(0.25);
    let dash = match ln.child(ns::A, "prstDash").and_then(|d| d.attr("val")) {
        Some("dash") => vec![4.0 * w, 3.0 * w],
        Some("dot" | "sysDot") => vec![w, w],
        Some("sysDash") => vec![3.0 * w, w],
        Some("dashDot" | "sysDashDot") => vec![4.0 * w, 3.0 * w, w, 3.0 * w],
        Some("lgDash") => vec![8.0 * w, 3.0 * w],
        Some("lgDashDot") => vec![8.0 * w, 3.0 * w, w, 3.0 * w],
        Some("lgDashDotDot" | "sysDashDotDot") => {
            vec![8.0 * w, 3.0 * w, w, 3.0 * w, w, 3.0 * w]
        }
        _ => Vec::new(),
    };
    Some(Stroke {
        paint,
        width,
        cap,
        join,
        dash,
    })
}

/// A theme style reference (`a:fillRef`, `a:lnRef`, `p:bgRef`): the style
/// at `idx` with `phClr` replaced by the reference's own colour.
pub fn style_ref<'t>(
    reference: &Element,
    list: &'t [Element],
    bg_list: &'t [Element],
    ctx: &ColorContext<'_>,
) -> Option<(&'t Element, Option<Rgba>)> {
    let idx: usize = reference.attr("idx")?.parse().ok()?;
    let color = child_color(reference, ctx);
    let style = match idx {
        0 => return None,
        1..=999 => list.get(idx - 1)?,
        _ => bg_list.get(idx - 1001)?,
    };
    Some((style, color))
}
