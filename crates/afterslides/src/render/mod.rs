//! Rendering slides to PDF and PNG (feature `render`).
//!
//! Slides are first resolved into a [`display::Page`] (all inheritance,
//! theme colours, geometry and text layout done), which the backends then
//! draw: [`pdf`] with krilla, [`raster`] with tiny-skia.
//!
//! This is a prototype: shapes, pictures, text and tables are drawn; charts,
//! SmartArt and effects are not yet (charts show a placeholder).

pub mod display;

mod chart;
mod color;
mod fill;
mod geom;
mod pdf;
mod raster;
mod scene;
mod shapes;
mod table;
mod text;

use std::collections::HashMap;

use crate::error::Result;
use crate::presentation::{Presentation, SlideId};

/// Options for [`Renderer`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RenderOptions {
    /// Also render slides marked as hidden (PowerPoint skips them in PDFs).
    pub include_hidden: bool,
    /// Extra directories with fonts (corporate fonts, Office fonts).
    pub font_dirs: Vec<std::path::PathBuf>,
    /// Font substitutions applied before lookup, e.g. Calibri -> Carlito.
    pub substitutions: HashMap<String, String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        let substitutions = [
            ("Calibri", "Carlito"),
            ("Calibri Light", "Carlito"),
            ("Cambria", "Caladea"),
            ("Arial", "Liberation Sans"),
            ("Helvetica", "Liberation Sans"),
            ("Times New Roman", "Liberation Serif"),
            ("Courier New", "Liberation Mono"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        RenderOptions {
            include_hidden: false,
            font_dirs: Vec::new(),
            substitutions,
        }
    }
}

/// Maps parley's font blobs to display-list font references, so each font
/// file is shared (not copied) and has a stable id.
#[derive(Default)]
pub(crate) struct FontIds {
    known: HashMap<(u64, u32), display::FontRef>,
}

impl FontIds {
    pub(crate) fn get(&mut self, font: &parley::FontData) -> display::FontRef {
        let key = (font.data.id(), font.index);
        self.known
            .entry(key)
            .or_insert_with(|| display::FontRef {
                data: std::sync::Arc::new(font.data.clone()),
                index: font.index,
                id: key.0.wrapping_mul(31).wrapping_add(u64::from(key.1)),
            })
            .clone()
    }
}

/// Renders presentations. Keep one around: it caches the system's fonts.
pub struct Renderer {
    pub(crate) options: RenderOptions,
    fonts: Option<parley::FontContext>,
    layout: parley::LayoutContext<display::Rgba>,
    pub(crate) font_ids: FontIds,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("options", &self.options)
            .finish()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer::new(RenderOptions::default())
    }
}

impl Renderer {
    pub fn new(options: RenderOptions) -> Renderer {
        Renderer {
            options,
            fonts: None,
            layout: parley::LayoutContext::new(),
            font_ids: FontIds::default(),
        }
    }

    /// Font and layout contexts; system fonts and the configured font
    /// directories are loaded on first use.
    pub(crate) fn contexts(
        &mut self,
    ) -> (
        &mut parley::FontContext,
        &mut parley::LayoutContext<display::Rgba>,
    ) {
        let dirs = &self.options.font_dirs;
        let fonts = self.fonts.get_or_insert_with(|| {
            let mut cx = parley::FontContext::new();
            for dir in dirs {
                register_dir(&mut cx, dir);
            }
            cx
        });
        (fonts, &mut self.layout)
    }

    pub fn options_mut(&mut self) -> &mut RenderOptions {
        &mut self.options
    }

    /// The display list of one slide.
    pub fn page(&mut self, prs: &Presentation, slide: SlideId) -> Result<display::Page> {
        scene::build(self, prs, slide)
    }

    /// Renders the deck (visible slides unless configured otherwise) to PDF.
    pub fn pdf(&mut self, prs: &Presentation) -> Result<Vec<u8>> {
        let mut pages = Vec::new();
        for slide in prs.slides()? {
            if !self.options.include_hidden && prs.slide_hidden(slide)? {
                continue;
            }
            pages.push(self.page(prs, slide)?);
        }
        pdf::write(&pages)
    }

    /// Renders one slide to PNG at `scale` pixels per point (1.0 = 72 dpi).
    pub fn png(&mut self, prs: &Presentation, slide: SlideId, scale: f64) -> Result<Vec<u8>> {
        let page = self.page(prs, slide)?;
        raster::png(&page, scale)
    }
}

fn register_dir(cx: &mut parley::FontContext, dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            register_dir(cx, &path);
            continue;
        }
        let is_font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        });
        if is_font && let Ok(bytes) = std::fs::read(&path) {
            let blob = parley::fontique::Blob::new(std::sync::Arc::new(bytes));
            cx.collection.register_fonts(blob, None);
        }
    }
}
