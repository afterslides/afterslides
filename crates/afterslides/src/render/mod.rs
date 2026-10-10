//! Rendering slides to PDF and PNG (feature `render`).
//!
//! Slides are first resolved into a [`display::Page`] (all inheritance,
//! theme colours, geometry and text layout done), which the backends then
//! draw: [`pdf`] with krilla, [`raster`] with tiny-skia.
//!
//! This is a prototype: shapes, pictures, text and tables are drawn; charts,
//! SmartArt and effects are not yet (charts show a placeholder).

pub mod display;

mod color;
mod fill;
mod geom;
mod pdf;
mod raster;
mod scene;
mod shapes;
mod stubs;

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

/// Renders presentations. Keep one around: it caches the system's fonts.
pub struct Renderer {
    options: RenderOptions,
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
        Renderer { options }
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
