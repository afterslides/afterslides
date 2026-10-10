//! Walking shape trees: master, layout and slide shapes, groups,
//! placeholders, fills, outlines and pictures.

use kurbo::{Affine, BezPath, Rect, Shape};

use super::color::ColorContext;
use super::display::{Item, Paint, Rgba};
use super::fill::{Fill, find_fill, resolve_fill, resolve_line, style_ref};
use super::geom::{PathFill, shape_geometry};
use super::scene::{EMU_PER_PT, Layer, Scene};
use crate::xml::{Element, ns};

/// Placeholder identity from `p:nvPr/p:ph`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Ph {
    pub kind: String,
    pub idx: Option<u32>,
}

fn nv_pr(shape: &Element) -> Option<&Element> {
    shape
        .elements()
        .find(|e| e.ns() == Some(ns::P) && e.local().starts_with("nv"))
        .and_then(|nv| nv.child(ns::P, "nvPr"))
}

pub(super) fn placeholder(shape: &Element) -> Option<Ph> {
    let ph = nv_pr(shape)?.child(ns::P, "ph")?;
    Some(Ph {
        kind: ph.attr("type").unwrap_or("body").to_string(),
        idx: ph.attr("idx").and_then(|v| v.parse().ok()),
    })
}

/// Placeholder types that inherit from one another across slide, layout and
/// master.
fn same_kind(a: &str, b: &str) -> bool {
    fn norm(k: &str) -> &str {
        match k {
            "ctrTitle" => "title",
            "subTitle" | "obj" => "body",
            other => other,
        }
    }
    norm(a) == norm(b)
}

/// The placeholder in `layer` that `ph` inherits from.
pub(super) fn find_placeholder<'a>(layer: Layer<'a>, ph: &Ph) -> Option<&'a Element> {
    let tree = layer.root.path(&[(ns::P, "cSld"), (ns::P, "spTree")])?;
    let candidates: Vec<(&Element, Ph)> = tree
        .elements()
        .filter_map(|e| placeholder(e).map(|p| (e, p)))
        .collect();
    if let Some(idx) = ph.idx
        && let Some((e, _)) = candidates.iter().find(|(_, p)| p.idx == Some(idx))
    {
        return Some(e);
    }
    candidates
        .iter()
        .find(|(_, p)| same_kind(&p.kind, &ph.kind))
        .map(|(e, _)| *e)
}

pub(super) fn sp_pr(shape: &Element) -> Option<&Element> {
    match shape.local() {
        "grpSp" => shape.child(ns::P, "grpSpPr"),
        _ => shape.child(ns::P, "spPr"),
    }
}

/// Position, size, rotation and flips of a shape.
#[derive(Debug, Clone, Copy)]
pub(super) struct Xfrm {
    pub rect: Rect,
    pub rot: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

pub(super) fn xfrm_of(xfrm: &Element) -> Option<Xfrm> {
    let num = |e: Option<&Element>, n: &str| {
        e.and_then(|e| e.attr(n))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
            / EMU_PER_PT
    };
    let off = xfrm.child(ns::A, "off");
    let ext = xfrm.child(ns::A, "ext");
    ext?;
    let (x, y) = (num(off, "x"), num(off, "y"));
    let (w, h) = (num(ext, "cx"), num(ext, "cy"));
    Some(Xfrm {
        rect: Rect::new(x, y, x + w, y + h),
        rot: xfrm
            .attr("rot")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 60000.0,
        flip_h: matches!(xfrm.attr("flipH"), Some("1" | "true")),
        flip_v: matches!(xfrm.attr("flipV"), Some("1" | "true")),
    })
}

impl Xfrm {
    /// Maps local shape coordinates (0..w, 0..h) onto the parent.
    pub fn affine(&self) -> Affine {
        let (w, h) = (self.rect.width(), self.rect.height());
        let center = Affine::translate((w / 2.0, h / 2.0));
        let flip = Affine::scale_non_uniform(
            if self.flip_h { -1.0 } else { 1.0 },
            if self.flip_v { -1.0 } else { 1.0 },
        );
        Affine::translate((self.rect.x0, self.rect.y0))
            * center
            * Affine::rotate(self.rot.to_radians())
            * flip
            * center.inverse()
    }
}

fn graphic_xfrm(frame: &Element) -> Option<Xfrm> {
    frame.child(ns::P, "xfrm").and_then(xfrm_of)
}

/// The elements a shape inherits from, nearest first: the shape itself,
/// then its layout and master placeholders.
pub(super) fn inheritance<'a>(
    scene: &Scene<'a, '_>,
    shape: &'a Element,
    from: &str,
) -> Vec<&'a Element> {
    let mut chain = vec![shape];
    let Some(ph) = placeholder(shape) else {
        return chain;
    };
    let layers = &scene.layers;
    let mut push = |layer: Option<Layer<'a>>| {
        if let Some(found) = layer.and_then(|l| find_placeholder(l, &ph)) {
            chain.push(found);
        }
    };
    match from {
        "slide" => {
            push(layers.layout);
            push(layers.master);
        }
        "layout" => push(layers.master),
        _ => {}
    }
    chain
}

fn mix(c: Rgba, with: Rgba, amount: f64) -> Rgba {
    let m = |a: u8, b: u8| (f64::from(a) * (1.0 - amount) + f64::from(b) * amount).round() as u8;
    Rgba {
        r: m(c.r, with.r),
        g: m(c.g, with.g),
        b: m(c.b, with.b),
        a: c.a,
    }
}

fn shade_paint(paint: &Paint, mode: PathFill) -> Paint {
    let adjust = |c: Rgba| match mode {
        PathFill::Lighten => mix(c, Rgba::WHITE, 0.4),
        PathFill::LightenLess => mix(c, Rgba::WHITE, 0.2),
        PathFill::Darken => mix(c, Rgba::BLACK, 0.4),
        PathFill::DarkenLess => mix(c, Rgba::BLACK, 0.2),
        _ => c,
    };
    match paint {
        Paint::Solid(c) => Paint::Solid(adjust(*c)),
        other => other.clone(),
    }
}

impl<'a> Scene<'a, '_> {
    pub(super) fn shapes(&mut self) {
        let layers = &self.layers;
        let hidden_master =
            |l: Layer<'_>| matches!(l.root.attr("showMasterSp"), Some("0" | "false"));
        let hide_layout_and_master = hidden_master(layers.slide);
        let hide_master = hide_layout_and_master || layers.layout.is_some_and(hidden_master);
        let mut todo: Vec<(Layer<'a>, &'static str)> = Vec::new();
        if !hide_master && let Some(m) = layers.master {
            todo.push((m, "master"));
        }
        if !hide_layout_and_master && let Some(l) = layers.layout {
            todo.push((l, "layout"));
        }
        todo.push((layers.slide, "slide"));

        for (layer, kind) in todo {
            let Some(tree) = layer.root.path(&[(ns::P, "cSld"), (ns::P, "spTree")]) else {
                continue;
            };
            // Placeholders on masters and layouts are templates, not content.
            let content_only = kind != "slide";
            self.tree(tree, layer, kind, Affine::IDENTITY, content_only);
        }
    }

    fn tree(
        &mut self,
        tree: &'a Element,
        layer: Layer<'a>,
        kind: &'static str,
        parent: Affine,
        content_only: bool,
    ) {
        for child in tree.elements() {
            if child.is(ns::MC, "AlternateContent") {
                // We don't know the extensions a Choice requires; the
                // Fallback is what other readers draw.
                let branch = child
                    .child(ns::MC, "Fallback")
                    .or_else(|| child.child(ns::MC, "Choice"));
                if let Some(branch) = branch {
                    self.tree(branch, layer, kind, parent, content_only);
                }
                continue;
            }
            if child.ns() != Some(ns::P) {
                continue;
            }
            if content_only && placeholder(child).is_some() {
                continue;
            }
            let hidden = child
                .elements()
                .find(|e| e.local().starts_with("nv"))
                .and_then(|nv| nv.child(ns::P, "cNvPr"))
                .is_some_and(|c| matches!(c.attr("hidden"), Some("1" | "true")));
            if hidden {
                continue;
            }
            match child.local() {
                "sp" | "cxnSp" => self.shape(child, layer, kind, parent),
                "pic" => self.picture(child, layer, kind, parent),
                "grpSp" => self.group(child, layer, kind, parent),
                "graphicFrame" => self.graphic_frame(child, layer, parent),
                _ => {}
            }
        }
    }

    fn group(&mut self, group: &'a Element, layer: Layer<'a>, kind: &'static str, parent: Affine) {
        let Some(xfrm) = group.path(&[(ns::P, "grpSpPr"), (ns::A, "xfrm")]) else {
            self.tree(group, layer, kind, parent, false);
            return;
        };
        let Some(outer) = xfrm_of(xfrm) else {
            self.tree(group, layer, kind, parent, false);
            return;
        };
        let num = |n: &str, a: &str| {
            xfrm.child(ns::A, n)
                .and_then(|e| e.attr(a))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / EMU_PER_PT
        };
        let (chx, chy) = (num("chOff", "x"), num("chOff", "y"));
        let (chw, chh) = (num("chExt", "cx"), num("chExt", "cy"));
        let sx = if chw > 0.0 {
            outer.rect.width() / chw
        } else {
            1.0
        };
        let sy = if chh > 0.0 {
            outer.rect.height() / chh
        } else {
            1.0
        };
        // Children live in the group's child coordinate space.
        let to_local = Affine::scale_non_uniform(sx, sy) * Affine::translate((-chx, -chy));
        let placed = Xfrm {
            rect: Rect::from_origin_size((0.0, 0.0), outer.rect.size()),
            ..outer
        };
        let transform =
            parent * Affine::translate((outer.rect.x0, outer.rect.y0)) * placed.affine() * to_local;
        self.tree(group, layer, kind, transform, false);
    }

    /// Resolves the fill of a shape: its own, then its style reference,
    /// then whatever its placeholders define.
    fn shape_fill(&self, chain: &[&Element], bounds: Rect) -> Option<Fill> {
        let colors = self.layers.colors();
        for el in chain {
            if let Some(fill) = sp_pr(el).and_then(find_fill) {
                return resolve_fill(fill, &colors, bounds);
            }
            if let Some(r) = el.path(&[(ns::P, "style"), (ns::A, "fillRef")]) {
                let theme = &self.layers.theme;
                let (style, color) =
                    style_ref(r, &theme.fill_styles, &theme.bg_fill_styles, &colors)?;
                return resolve_fill(style, &colors.with_placeholder(color), bounds);
            }
        }
        None
    }

    fn shape_line(&self, chain: &[&Element], bounds: Rect) -> Option<super::display::Stroke> {
        let colors = self.layers.colors();
        // The theme style is the base; the shape's own a:ln overrides it.
        let own = chain
            .iter()
            .find_map(|el| sp_pr(el).and_then(|p| p.child(ns::A, "ln")));
        let styled = chain.iter().find_map(|el| {
            let r = el.path(&[(ns::P, "style"), (ns::A, "lnRef")])?;
            let theme = &self.layers.theme;
            style_ref(r, &theme.line_styles, &[], &colors)
        });
        let (base, ctx): (Option<&Element>, ColorContext<'_>) = match &styled {
            Some((style, color)) => (Some(*style), colors.with_placeholder(*color)),
            None => (None, colors),
        };
        match (own, base) {
            (Some(own), Some(base)) => {
                // Merge: attributes and children of the shape's line win.
                let mut merged = base.clone();
                for a in &own.attrs {
                    merged.set_attr(&a.name, a.value.clone());
                }
                if find_fill(own).is_some() {
                    merged.children.retain(
                        |n| !matches!(n, crate::xml::Node::Element(e) if super::fill::is_fill(e)),
                    );
                }
                for child in own.elements() {
                    let name = child.local().to_string();
                    merged.children.retain(
                        |n| !matches!(n, crate::xml::Node::Element(e) if e.local() == name),
                    );
                    merged
                        .children
                        .push(crate::xml::Node::Element(child.clone()));
                }
                resolve_line(&merged, &ctx, bounds)
            }
            (Some(own), None) => resolve_line(own, &colors, bounds),
            (None, Some(base)) => resolve_line(base, &ctx, bounds),
            (None, None) => None,
        }
    }

    fn placement(&self, chain: &[&Element]) -> Option<Xfrm> {
        chain.iter().find_map(|el| {
            sp_pr(el)
                .and_then(|p| p.child(ns::A, "xfrm"))
                .and_then(xfrm_of)
        })
    }

    fn shape(&mut self, shape: &'a Element, layer: Layer<'a>, kind: &'static str, parent: Affine) {
        let chain = inheritance(self, shape, kind);
        let Some(xfrm) = self.placement(&chain) else {
            return;
        };
        let (w, h) = (xfrm.rect.width(), xfrm.rect.height());
        let transform = parent * xfrm.affine();
        let geometry = shape_geometry(sp_pr(shape), w, h);
        let bounds = Rect::new(0.0, 0.0, w, h);
        let fill = self.shape_fill(&chain, bounds);
        let line = self.shape_line(&chain, bounds);

        for sub in &geometry.paths {
            if sub.fill == PathFill::None {
                continue;
            }
            match &fill {
                Some(Fill::Paint(paint)) => self.items.push(Item::Fill {
                    path: sub.path.clone(),
                    paint: shade_paint(paint, sub.fill),
                    transform,
                    even_odd: false,
                }),
                Some(other) => {
                    let part = layer.part.to_string();
                    self.draw_fill(other.clone(), &part, sub.path.clone(), bounds, transform);
                }
                None => {}
            }
        }
        if let Some(stroke) = &line {
            for sub in geometry.paths.iter().filter(|s| s.stroke) {
                self.items.push(Item::Stroke {
                    path: sub.path.clone(),
                    stroke: stroke.clone(),
                    transform,
                });
            }
        }
        self.text_frame(&chain, geometry.text_rect, transform, xfrm);
    }

    fn picture(&mut self, pic: &'a Element, layer: Layer<'a>, kind: &'static str, parent: Affine) {
        let chain = inheritance(self, pic, kind);
        let Some(xfrm) = self.placement(&chain) else {
            return;
        };
        let (w, h) = (xfrm.rect.width(), xfrm.rect.height());
        let transform = parent * xfrm.affine();
        let bounds = Rect::new(0.0, 0.0, w, h);
        let geometry = shape_geometry(sp_pr(pic), w, h);
        let outline: BezPath = geometry
            .paths
            .first()
            .map_or_else(|| bounds.to_path(0.1), |p| p.path.clone());
        let blip_fill = pic.child(ns::P, "blipFill").or_else(|| {
            pic.child(ns::MC, "AlternateContent")
                .and_then(|ac| {
                    ac.child(ns::MC, "Fallback")
                        .or_else(|| ac.child(ns::MC, "Choice"))
                })
                .and_then(|b| b.child(ns::P, "blipFill"))
        });
        if let Some(fill) = blip_fill
            && let Some(rid) = fill
                .child(ns::A, "blip")
                .and_then(|b| b.attr_ns(ns::R, "embed"))
        {
            let crop = super::fill::crop_of(fill);
            let part = layer.part.to_string();
            self.draw_fill(
                Fill::Picture {
                    rid: rid.to_string(),
                    crop,
                },
                &part,
                outline.clone(),
                bounds,
                transform,
            );
        }
        if let Some(stroke) = self.shape_line(&chain, bounds) {
            self.items.push(Item::Stroke {
                path: outline,
                stroke,
                transform,
            });
        }
    }

    fn graphic_frame(&mut self, frame: &'a Element, layer: Layer<'a>, parent: Affine) {
        let Some(xfrm) = graphic_xfrm(frame) else {
            return;
        };
        let transform = parent * xfrm.affine();
        let bounds = Rect::new(0.0, 0.0, xfrm.rect.width(), xfrm.rect.height());
        let data = frame.path(&[(ns::A, "graphic"), (ns::A, "graphicData")]);
        match data.and_then(|d| d.attr("uri")) {
            Some("http://schemas.openxmlformats.org/drawingml/2006/table") => {
                if let Some(tbl) = data.and_then(|d| d.child(ns::A, "tbl")) {
                    self.table(tbl, layer, bounds, transform);
                }
            }
            Some("http://schemas.openxmlformats.org/drawingml/2006/chart") => {
                self.chart_frame(frame, layer, bounds, transform);
            }
            _ => self.unsupported_box(bounds, transform),
        }
    }

    /// A light box where we can't draw the content yet.
    pub(super) fn unsupported_box(&mut self, bounds: Rect, transform: Affine) {
        let path = bounds.to_path(0.1);
        self.items.push(Item::Fill {
            path: path.clone(),
            paint: Paint::Solid(Rgba::rgb(0xF2, 0xF2, 0xF2)),
            transform,
            even_odd: false,
        });
        self.items.push(Item::Stroke {
            path,
            stroke: super::display::Stroke {
                paint: Paint::Solid(Rgba::rgb(0xBF, 0xBF, 0xBF)),
                width: 0.75,
                cap: super::display::LineCap::Butt,
                join: super::display::LineJoin::Miter,
                dash: vec![3.0, 2.0],
            },
            transform,
        });
    }
}
