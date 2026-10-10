//! PNG backend (tiny-skia). Glyphs are drawn from their outlines (skrifa).

use std::collections::HashMap;

use kurbo::{Affine, BezPath, PathEl};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use tiny_skia as sk;

use super::display::{
    GlyphRun, ImageData, ImageFormat, Item, LineCap, LineJoin, Page, Paint, Rgba, Stroke,
};
use crate::error::{Error, Result};

pub fn png(page: &Page, scale: f64) -> Result<Vec<u8>> {
    let pixmap = rasterize(page, scale)?;
    pixmap
        .encode_png()
        .map_err(|e| Error::Unsupported(format!("PNG encoding failed: {e}")))
}

pub fn rasterize(page: &Page, scale: f64) -> Result<sk::Pixmap> {
    let w = (page.width * scale).round().max(1.0) as u32;
    let h = (page.height * scale).round().max(1.0) as u32;
    let mut pixmap = sk::Pixmap::new(w, h)
        .ok_or_else(|| Error::InvalidArgument(format!("cannot render a {w}x{h} image")))?;
    pixmap.fill(sk::Color::WHITE);
    let base = Affine::scale(scale);
    let mut clips: Vec<sk::Mask> = Vec::new();
    let mut images: HashMap<*const ImageData, Option<sk::Pixmap>> = HashMap::new();

    for item in &page.items {
        let mask = clips.last();
        match item {
            Item::Fill {
                path,
                paint,
                transform,
                even_odd,
            } => {
                let Some(path) = to_path(path) else { continue };
                let t = to_transform(base * *transform);
                let Some(paint) = to_paint(paint) else {
                    continue;
                };
                let rule = if *even_odd {
                    sk::FillRule::EvenOdd
                } else {
                    sk::FillRule::Winding
                };
                pixmap.fill_path(&path, &paint, rule, t, mask);
            }
            Item::Stroke {
                path,
                stroke,
                transform,
            } => {
                let Some(path) = to_path(path) else { continue };
                let t = to_transform(base * *transform);
                let Some(paint) = to_paint(&stroke.paint) else {
                    continue;
                };
                pixmap.stroke_path(&path, &paint, &to_stroke(stroke), t, mask);
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
                    .or_insert_with(|| decode(image));
                let Some(img) = decoded else { continue };
                let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
                // Visible part of the image after cropping, in pixels.
                let (cl, ct) = (crop[0] * iw, crop[1] * ih);
                let cw = (iw * (1.0 - crop[0] - crop[2])).max(1.0);
                let ch = (ih * (1.0 - crop[1] - crop[3])).max(1.0);
                let image_to_rect = Affine::translate((rect.x0, rect.y0))
                    * Affine::scale_non_uniform(rect.width() / cw, rect.height() / ch)
                    * Affine::translate((-cl, -ct));
                let t = to_transform(base * *transform * image_to_rect);
                let shader = sk::Pattern::new(
                    img.as_ref(),
                    sk::SpreadMode::Pad,
                    sk::FilterQuality::Bilinear,
                    *opacity,
                    t,
                );
                let paint = sk::Paint {
                    shader,
                    anti_alias: true,
                    ..Default::default()
                };
                let area = kurbo::Rect::from_origin_size((0.0, 0.0), rect.size())
                    + kurbo::Vec2::new(rect.x0, rect.y0);
                if let Some(path) = to_path(&kurbo::Shape::to_path(&area, 0.1)) {
                    pixmap.fill_path(
                        &path,
                        &paint,
                        sk::FillRule::Winding,
                        to_transform(base * *transform),
                        mask,
                    );
                }
            }
            Item::Glyphs { run, transform } => {
                draw_glyphs(&mut pixmap, run, base * *transform, mask);
            }
            Item::PushClip { path, transform } => {
                let Some(path) = to_path(path) else {
                    // An empty clip hides everything until the matching pop.
                    clips.push(sk::Mask::new(w, h).expect("non-zero size"));
                    continue;
                };
                let t = to_transform(base * *transform);
                let new = match clips.last() {
                    Some(outer) => {
                        let mut m = outer.clone();
                        m.intersect_path(&path, sk::FillRule::Winding, true, t);
                        m
                    }
                    None => {
                        let mut m = sk::Mask::new(w, h).expect("non-zero size");
                        m.fill_path(&path, sk::FillRule::Winding, true, t);
                        m
                    }
                };
                clips.push(new);
            }
            Item::PopClip => {
                clips.pop();
            }
        }
    }
    Ok(pixmap)
}

fn decode(image: &ImageData) -> Option<sk::Pixmap> {
    match image.format {
        ImageFormat::Png => sk::Pixmap::decode_png(&image.bytes).ok(),
        ImageFormat::Jpeg => {
            use zune_jpeg::JpegDecoder;
            use zune_jpeg::zune_core::bytestream::ZCursor;
            use zune_jpeg::zune_core::colorspace::ColorSpace;
            use zune_jpeg::zune_core::options::DecoderOptions;
            let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
            let mut decoder = JpegDecoder::new_with_options(ZCursor::new(&image.bytes), options);
            let pixels = decoder.decode().ok()?;
            let (w, h) = decoder.dimensions()?;
            sk::Pixmap::from_vec(pixels, sk::IntSize::from_wh(w as u32, h as u32)?)
        }
    }
}

pub(super) fn to_path(path: &BezPath) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
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

fn to_transform(a: Affine) -> sk::Transform {
    let [sx, ky, kx, sy, tx, ty] = a.as_coeffs();
    sk::Transform::from_row(
        sx as f32, ky as f32, kx as f32, sy as f32, tx as f32, ty as f32,
    )
}

fn to_color(c: Rgba) -> sk::Color {
    sk::Color::from_rgba8(c.r, c.g, c.b, (c.a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn to_paint(paint: &Paint) -> Option<sk::Paint<'static>> {
    let stops = |stops: &[super::display::GradientStop]| {
        stops
            .iter()
            .map(|s| sk::GradientStop::new(s.offset, to_color(s.color)))
            .collect::<Vec<_>>()
    };
    let shader = match paint {
        Paint::Solid(c) => sk::Shader::SolidColor(to_color(*c)),
        Paint::Linear {
            start,
            end,
            stops: s,
        } => sk::LinearGradient::new(
            sk::Point::from_xy(start.x as f32, start.y as f32),
            sk::Point::from_xy(end.x as f32, end.y as f32),
            stops(s),
            sk::SpreadMode::Pad,
            sk::Transform::identity(),
        )?,
        Paint::Radial {
            center,
            radius,
            stops: s,
        } => {
            let c = sk::Point::from_xy(center.x as f32, center.y as f32);
            sk::RadialGradient::new(
                c,
                0.0,
                c,
                *radius as f32,
                stops(s),
                sk::SpreadMode::Pad,
                sk::Transform::identity(),
            )?
        }
    };
    Some(sk::Paint {
        shader,
        anti_alias: true,
        ..Default::default()
    })
}

fn to_stroke(s: &Stroke) -> sk::Stroke {
    sk::Stroke {
        width: s.width as f32,
        line_cap: match s.cap {
            LineCap::Butt => sk::LineCap::Butt,
            LineCap::Round => sk::LineCap::Round,
            LineCap::Square => sk::LineCap::Square,
        },
        line_join: match s.join {
            LineJoin::Miter => sk::LineJoin::Miter,
            LineJoin::Round => sk::LineJoin::Round,
            LineJoin::Bevel => sk::LineJoin::Bevel,
        },
        dash: if s.dash.len() >= 2 {
            sk::StrokeDash::new(s.dash.iter().map(|d| *d as f32).collect(), 0.0)
        } else {
            None
        },
        ..Default::default()
    }
}

/// Collects a glyph outline as a kurbo path, flipping y (fonts are y-up).
struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), -f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), -f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(x), -f64::from(y)),
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(cx1), -f64::from(cy1)),
            (f64::from(x), -f64::from(y)),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

fn draw_glyphs(
    pixmap: &mut sk::Pixmap,
    run: &GlyphRun,
    transform: Affine,
    mask: Option<&sk::Mask>,
) {
    let Ok(font) = skrifa::FontRef::from_index(&run.font.data, run.font.index) else {
        return;
    };
    let outlines = font.outline_glyphs();
    let Some(paint) = to_paint(&Paint::Solid(run.color)) else {
        return;
    };
    let size = Size::new(run.size as f32);
    for glyph in &run.glyphs {
        let Some(outline) = outlines.get(skrifa::GlyphId::new(glyph.id)) else {
            continue;
        };
        let mut pen = Pen(BezPath::new());
        if outline
            .draw(
                DrawSettings::unhinted(size, LocationRef::default()),
                &mut pen,
            )
            .is_err()
        {
            continue;
        }
        let Some(path) = to_path(&pen.0) else {
            continue;
        };
        let t = to_transform(transform * Affine::translate((glyph.x, glyph.y)));
        pixmap.fill_path(&path, &paint, sk::FillRule::Winding, t, mask);
    }
}
