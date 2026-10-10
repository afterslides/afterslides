//! A backend-neutral list of drawing operations for one slide.
//!
//! Resolving PresentationML (inheritance, themes, geometry, text layout)
//! produces these items; the PDF and PNG backends only draw them. Units are
//! points with the origin at the top left of the slide.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0.0 (transparent) to 1.0 (opaque).
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    /// 0.0 to 1.0 along the gradient.
    pub offset: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Rgba),
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    Radial {
        center: Point,
        radius: f64,
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Dash pattern in points (on, off, on, ...); empty for a solid line.
    pub dash: Vec<f64>,
}

/// Decoded or encoded image data shared between items.
#[derive(Debug)]
pub struct ImageData {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// A font as raw bytes plus the face index inside a collection.
#[derive(Debug, Clone)]
pub struct FontRef {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    /// Stable identity for caches in the backends.
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub id: u32,
    /// Pen position in points.
    pub x: f64,
    pub y: f64,
    /// Horizontal advance in points.
    pub advance: f64,
}

#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub font: FontRef,
    pub size: f64,
    pub color: Rgba,
    pub glyphs: Vec<Glyph>,
    /// The text the glyphs came from, for selectable PDF text.
    pub text: String,
    /// For each glyph, the byte range in `text` it covers.
    pub clusters: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Fill {
        path: BezPath,
        paint: Paint,
        transform: Affine,
        even_odd: bool,
    },
    Stroke {
        path: BezPath,
        stroke: Stroke,
        transform: Affine,
    },
    Image {
        image: Arc<ImageData>,
        /// Where the (cropped) image goes, in local coordinates.
        rect: Rect,
        /// Crop as fractions of the image: left, top, right, bottom.
        crop: [f64; 4],
        transform: Affine,
        opacity: f32,
    },
    Glyphs {
        run: GlyphRun,
        transform: Affine,
    },
    /// Clip following items to a path until the matching `PopClip`.
    PushClip {
        path: BezPath,
        transform: Affine,
    },
    PopClip,
}

#[derive(Debug, Clone)]
pub struct Page {
    /// Size in points.
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}
