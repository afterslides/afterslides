//! Turns a slide, with its layout, master and theme, into a display list.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Rect, Shape};

use super::Renderer;
use super::color::{ColorContext, Theme, default_color_map, parse_color_map};
use super::display::{ImageData, ImageFormat, Item, Page, Paint, Rgba};
use super::fill::{Fill, find_fill, resolve_fill, style_ref};
use crate::error::Result;
use crate::presentation::{Presentation, SlideId};
use crate::xml::{Element, ns};

pub(super) const EMU_PER_PT: f64 = 12700.0;

const REL_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
const REL_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

/// A slide together with the parts it inherits from.
pub(super) struct Layers<'a> {
    pub prs: &'a Presentation,
    pub slide: Layer<'a>,
    pub layout: Option<Layer<'a>>,
    pub master: Option<Layer<'a>>,
    pub theme: Theme,
    pub color_map: HashMap<String, String>,
}

#[derive(Clone, Copy)]
pub(super) struct Layer<'a> {
    pub part: &'a str,
    pub root: &'a Element,
}

impl<'a> Layers<'a> {
    fn load(prs: &'a Presentation, slide_part: &'a str) -> Result<Layers<'a>> {
        let target = |source: &str, rel: &str| -> Option<String> {
            prs.pkg
                .targets(source)
                .into_iter()
                .find(|(r, t)| r.rel_type == rel && prs.pkg.has_part(t))
                .map(|(_, t)| t)
        };
        // Leak-free part name storage: names live as long as `prs`.
        let part_name = |name: Option<String>| -> Option<&'a str> {
            let name = name?;
            prs.pkg.part_names().find(|p| *p == name)
        };
        let layout_part = part_name(target(slide_part, REL_LAYOUT));
        let master_part = layout_part.and_then(|l| part_name(target(l, REL_MASTER)));
        let theme_part = master_part.and_then(|m| part_name(target(m, REL_THEME)));

        let layer = |part: &'a str| -> Result<Layer<'a>> {
            Ok(Layer {
                part,
                root: &prs.pkg.xml(part)?.root,
            })
        };
        let slide = layer(slide_part)?;
        let layout = layout_part.map(layer).transpose()?;
        let master = master_part.map(layer).transpose()?;
        let theme = match theme_part {
            Some(p) => Theme::parse(&prs.pkg.xml(p)?.root),
            None => Theme::default(),
        };
        // The slide (or layout) may override the master's colour map.
        let override_map = [Some(slide), layout].into_iter().flatten().find_map(|l| {
            l.root
                .path(&[(ns::P, "clrMapOvr"), (ns::A, "overrideClrMapping")])
        });
        let color_map = override_map
            .or_else(|| master.and_then(|m| m.root.child(ns::P, "clrMap")))
            .map(parse_color_map)
            .unwrap_or_else(default_color_map);
        Ok(Layers {
            prs,
            slide,
            layout,
            master,
            theme,
            color_map,
        })
    }

    pub fn colors(&self) -> ColorContext<'_> {
        ColorContext {
            theme: &self.theme,
            map: &self.color_map,
            placeholder: None,
        }
    }

    /// Slide, layout, master: the order in which properties are looked up.
    pub fn chain(&self) -> impl Iterator<Item = Layer<'a>> + '_ {
        [Some(self.slide), self.layout, self.master]
            .into_iter()
            .flatten()
    }

    /// Image bytes behind relationship `rid` of `part`, if PNG or JPEG.
    pub fn image(&self, part: &str, rid: &str) -> Option<Arc<ImageData>> {
        let target = self.prs.pkg.resolve(part, rid).ok()?;
        let bytes = self.prs.pkg.raw(&target).ok()?;
        let format = match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => ImageFormat::Png,
            [0xFF, 0xD8, ..] => ImageFormat::Jpeg,
            _ => return None,
        };
        Some(Arc::new(ImageData { bytes, format }))
    }
}

/// Collects display items for one slide.
pub(super) struct Scene<'a, 'r> {
    pub layers: Layers<'a>,
    #[allow(dead_code)]
    pub renderer: &'r mut Renderer,
    pub items: Vec<Item>,
    pub width: f64,
    pub height: f64,
}

pub(super) fn build(renderer: &mut Renderer, prs: &Presentation, slide: SlideId) -> Result<Page> {
    let (cx, cy) = prs.slide_size()?;
    let slide_part = prs.slide_part(slide)?;
    let slide_part =
        prs.pkg
            .part_names()
            .find(|p| *p == slide_part)
            .ok_or(crate::error::Error::NotFound(crate::error::Target::Slide(
                slide.0,
            )))?;
    let layers = Layers::load(prs, slide_part)?;
    let mut scene = Scene {
        layers,
        renderer,
        items: Vec::new(),
        width: cx as f64 / EMU_PER_PT,
        height: cy as f64 / EMU_PER_PT,
    };
    scene.background();
    Ok(Page {
        width: scene.width,
        height: scene.height,
        items: scene.items,
    })
}

impl Scene<'_, '_> {
    fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width, self.height)
    }

    /// Draws a resolved fill into `path` (in local coordinates).
    pub fn draw_fill(
        &mut self,
        fill: Fill,
        part: &str,
        path: BezPath,
        bounds: Rect,
        transform: Affine,
    ) {
        match fill {
            Fill::None => {}
            Fill::Paint(paint) => self.items.push(Item::Fill {
                path,
                paint,
                transform,
                even_odd: false,
            }),
            Fill::Picture { rid, crop } => match self.layers.image(part, &rid) {
                Some(image) => {
                    self.items.push(Item::PushClip { path, transform });
                    self.items.push(Item::Image {
                        image,
                        rect: bounds,
                        crop,
                        transform,
                        opacity: 1.0,
                    });
                    self.items.push(Item::PopClip);
                }
                None => self.items.push(Item::Fill {
                    path,
                    paint: Paint::Solid(Rgba::rgb(0xD9, 0xD9, 0xD9)),
                    transform,
                    even_odd: false,
                }),
            },
        }
    }

    fn background(&mut self) {
        let bounds = self.bounds();
        let rect = bounds.to_path(0.1);
        let colors = self.layers.colors();
        let found = self.layers.chain().find_map(|layer| {
            let bg = layer.root.path(&[(ns::P, "cSld"), (ns::P, "bg")])?;
            if let Some(props) = bg.child(ns::P, "bgPr") {
                let fill = resolve_fill(find_fill(props)?, &colors, bounds)?;
                return Some((fill, layer.part));
            }
            let reference = bg.child(ns::P, "bgRef")?;
            let theme = &self.layers.theme;
            let (style, color) = style_ref(
                reference,
                &theme.fill_styles,
                &theme.bg_fill_styles,
                &colors,
            )?;
            let fill = resolve_fill(style, &colors.with_placeholder(color), bounds)?;
            Some((fill, layer.part))
        });
        match found {
            Some((fill, part)) => {
                // White underneath, like paper; matters for transparent fills.
                self.items.push(Item::Fill {
                    path: rect.clone(),
                    paint: Paint::Solid(Rgba::WHITE),
                    transform: Affine::IDENTITY,
                    even_odd: false,
                });
                let part = part.to_string();
                self.draw_fill(fill, &part, rect, bounds, Affine::IDENTITY);
            }
            None => self.items.push(Item::Fill {
                path: rect,
                paint: Paint::Solid(Rgba::WHITE),
                transform: Affine::IDENTITY,
                even_odd: false,
            }),
        }
    }
}
