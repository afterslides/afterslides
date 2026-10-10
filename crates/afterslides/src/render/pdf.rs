//! PDF backend (krilla). Text stays real, selectable text with embedded,
//! subsetted fonts.

use std::collections::HashMap;

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{Path, PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{
    Fill, FillRule, LineCap as KCap, LineJoin as KJoin, LinearGradient, RadialGradient,
    SpreadMethod, Stop, Stroke as KStroke, StrokeDash,
};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use kurbo::{Affine, BezPath, PathEl};

use super::display::{
    GlyphRun, ImageData, ImageFormat, Item, LineCap, LineJoin, Page, Paint, Rgba, Stroke,
};
use crate::error::{Error, Result};

pub fn write(pages: &[Page]) -> Result<Vec<u8>> {
    let mut document = Document::new();
    let mut fonts: HashMap<u64, Option<Font>> = HashMap::new();
    let mut images: HashMap<*const ImageData, Option<Image>> = HashMap::new();
    for page in pages {
        let settings = PageSettings::from_wh(page.width as f32, page.height as f32)
            .ok_or_else(|| Error::InvalidArgument("slide has no size".into()))?;
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        for item in &page.items {
            draw(&mut surface, item, &mut fonts, &mut images);
        }
        surface.finish();
        pdf_page.finish();
    }
    document
        .finish()
        .map_err(|e| Error::Unsupported(format!("PDF export failed: {e:?}")))
}

fn draw(
    surface: &mut Surface<'_>,
    item: &Item,
    fonts: &mut HashMap<u64, Option<Font>>,
    images: &mut HashMap<*const ImageData, Option<Image>>,
) {
    match item {
        Item::Fill {
            path,
            paint,
            transform,
            even_odd,
        } => {
            let Some(path) = to_path(path) else { return };
            surface.push_transform(&to_transform(*transform));
            surface.set_stroke(None);
            surface.set_fill(Some(to_fill(paint, *even_odd)));
            surface.draw_path(&path);
            surface.pop();
        }
        Item::Stroke {
            path,
            stroke,
            transform,
        } => {
            let Some(path) = to_path(path) else { return };
            surface.push_transform(&to_transform(*transform));
            surface.set_fill(None);
            surface.set_stroke(Some(to_stroke(stroke)));
            surface.draw_path(&path);
            surface.set_stroke(None);
            surface.pop();
        }
        Item::Image {
            image,
            rect,
            crop,
            transform,
            opacity,
        } => {
            let decoded = images
                .entry(std::sync::Arc::as_ptr(image))
                .or_insert_with(|| {
                    let data: krilla::Data = image.bytes.clone().into();
                    match image.format {
                        ImageFormat::Png => Image::from_png(data, true).ok(),
                        ImageFormat::Jpeg => Image::from_jpeg(data, true).ok(),
                    }
                })
                .clone();
            let Some(img) = decoded else { return };
            // Draw the whole image scaled so that the uncropped part fills
            // `rect`, clipped to `rect`.
            let visible_w = (1.0 - crop[0] - crop[2]).max(1e-6);
            let visible_h = (1.0 - crop[1] - crop[3]).max(1e-6);
            let full_w = rect.width() / visible_w;
            let full_h = rect.height() / visible_h;
            let origin = (rect.x0 - crop[0] * full_w, rect.y0 - crop[1] * full_h);
            let Some(clip) = to_path(&kurbo::Shape::to_path(rect, 0.1)) else {
                return;
            };
            let Some(size) = Size::from_wh(full_w as f32, full_h as f32) else {
                return;
            };
            surface.push_transform(&to_transform(*transform));
            surface.push_clip_path(&clip, &FillRule::NonZero);
            if *opacity < 1.0 {
                surface.push_opacity(
                    NormalizedF32::new(opacity.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE),
                );
            }
            surface.push_transform(&Transform::from_translate(origin.0 as f32, origin.1 as f32));
            surface.draw_image(img, size);
            surface.pop();
            if *opacity < 1.0 {
                surface.pop();
            }
            surface.pop();
            surface.pop();
        }
        Item::Glyphs { run, transform } => {
            let font = fonts
                .entry(run.font.id)
                .or_insert_with(|| Font::new(run.font.data.clone().into(), run.font.index))
                .clone();
            if let Some(font) = font {
                surface.push_transform(&to_transform(*transform));
                draw_glyphs(surface, run, font);
                surface.pop();
            }
        }
        Item::PushClip { path, transform } => {
            // Clip paths are specified in their own transform; the clip
            // stays active after the transform is popped.
            let mut placed = path.clone();
            placed.apply_affine(*transform);
            match to_path(&placed) {
                Some(path) => surface.push_clip_path(&path, &FillRule::NonZero),
                None => {
                    // Degenerate clip: clip to an empty rectangle.
                    let mut pb = PathBuilder::new();
                    pb.move_to(0.0, 0.0);
                    pb.line_to(0.0, 0.0001);
                    pb.line_to(0.0001, 0.0);
                    pb.close();
                    if let Some(p) = pb.finish() {
                        surface.push_clip_path(&p, &FillRule::NonZero);
                    }
                }
            }
        }
        Item::PopClip => surface.pop(),
    }
}

fn draw_glyphs(surface: &mut Surface<'_>, run: &GlyphRun, font: Font) {
    let Some(first) = run.glyphs.first() else {
        return;
    };
    let size = run.size as f32;
    let start = Point::from_xy(first.x as f32, first.y as f32);
    let glyphs: Vec<KrillaGlyph> = run
        .glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            // Advance to the next glyph's pen position; the last one uses
            // its own advance. Units are relative to the font size.
            let advance = match run.glyphs.get(i + 1) {
                Some(next) => next.x - g.x,
                None => g.advance,
            };
            KrillaGlyph::new(
                GlyphId::new(g.id),
                (advance / run.size) as f32,
                0.0,
                ((g.y - first.y) / run.size) as f32,
                0.0,
                run.clusters.get(i).cloned().unwrap_or(0..0),
                None,
            )
        })
        .collect();
    surface.set_stroke(None);
    surface.set_fill(Some(to_fill(&Paint::Solid(run.color), false)));
    surface.draw_glyphs(start, &glyphs, font, &run.text, size, false);
}

fn to_path(path: &BezPath) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => pb.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, b, p) => pb.cubic_to(
                a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32,
            ),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

fn to_transform(a: Affine) -> Transform {
    let [sx, ky, kx, sy, tx, ty] = a.as_coeffs();
    Transform::from_row(
        sx as f32, ky as f32, kx as f32, sy as f32, tx as f32, ty as f32,
    )
}

fn rgb_of(c: Rgba) -> rgb::Color {
    rgb::Color::new(c.r, c.g, c.b)
}

fn opacity(a: f32) -> NormalizedF32 {
    NormalizedF32::new(a.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn stops(stops: &[super::display::GradientStop]) -> Vec<Stop> {
    stops
        .iter()
        .map(|s| Stop {
            offset: NormalizedF32::new(s.offset.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ZERO),
            color: rgb_of(s.color).into(),
            opacity: opacity(s.color.a),
        })
        .collect()
}

fn to_paint(paint: &Paint) -> (krilla::paint::Paint, NormalizedF32) {
    match paint {
        Paint::Solid(c) => (rgb_of(*c).into(), opacity(c.a)),
        Paint::Linear {
            start,
            end,
            stops: s,
        } => (
            LinearGradient {
                x1: start.x as f32,
                y1: start.y as f32,
                x2: end.x as f32,
                y2: end.y as f32,
                transform: Transform::identity(),
                spread_method: SpreadMethod::Pad,
                stops: stops(s),
                anti_alias: true,
            }
            .into(),
            NormalizedF32::ONE,
        ),
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => (
            RadialGradient {
                fx: center.x as f32,
                fy: center.y as f32,
                fr: 0.0,
                cx: center.x as f32,
                cy: center.y as f32,
                cr: *radius as f32,
                transform: Transform::identity(),
                spread_method: SpreadMethod::Pad,
                stops: stops(s),
                anti_alias: true,
            }
            .into(),
            NormalizedF32::ONE,
        ),
    }
}

fn to_fill(paint: &Paint, even_odd: bool) -> Fill {
    let (paint, opacity) = to_paint(paint);
    Fill {
        paint,
        opacity,
        rule: if even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        },
    }
}

fn to_stroke(s: &Stroke) -> KStroke {
    let (paint, opacity) = to_paint(&s.paint);
    KStroke {
        paint,
        opacity,
        width: s.width as f32,
        miter_limit: 10.0,
        line_cap: match s.cap {
            LineCap::Butt => KCap::Butt,
            LineCap::Round => KCap::Round,
            LineCap::Square => KCap::Square,
        },
        line_join: match s.join {
            LineJoin::Miter => KJoin::Miter,
            LineJoin::Round => KJoin::Round,
            LineJoin::Bevel => KJoin::Bevel,
        },
        dash: (s.dash.len() >= 2).then(|| StrokeDash {
            array: s.dash.iter().map(|d| *d as f32).collect(),
            offset: 0.0,
        }),
    }
}
