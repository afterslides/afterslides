//! Replacing the image of a picture (logos, product shots, maps, ...).

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::opc::relative_target;
use crate::presentation::{Presentation, ShapeRef};
use crate::shape::{kind_of, rel_ids};
use crate::xml::{Attr, Element, Node, ns};

const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// How a new image is fitted into the picture's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Fit {
    /// Fill the frame exactly; the image is distorted if aspect ratios differ.
    #[default]
    Stretch,
    /// Shrink the frame to the image's aspect ratio, centred in the old frame.
    Contain,
    /// Keep the frame and crop the image to fill it.
    Cover,
}

struct ImageInfo {
    extension: &'static str,
    content_type: &'static str,
    /// Pixel size, when the format makes it easy to read.
    size: Option<(u32, u32)>,
}

fn sniff(data: &[u8]) -> Result<ImageInfo> {
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let le16 = |b: &[u8]| u32::from(u16::from_le_bytes([b[0], b[1]]));
    let info = |extension, content_type, size| ImageInfo {
        extension,
        content_type,
        size,
    };
    Ok(match data {
        [0x89, b'P', b'N', b'G', ..] if data.len() >= 24 => info(
            "png",
            "image/png",
            Some((be32(&data[16..20]), be32(&data[20..24]))),
        ),
        [0xFF, 0xD8, ..] => info("jpeg", "image/jpeg", jpeg_size(data)),
        [b'G', b'I', b'F', b'8', ..] if data.len() >= 10 => info(
            "gif",
            "image/gif",
            Some((le16(&data[6..8]), le16(&data[8..10]))),
        ),
        [b'B', b'M', ..] if data.len() >= 26 => {
            let w = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
            let h = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
            info(
                "bmp",
                "image/bmp",
                Some((w.unsigned_abs(), h.unsigned_abs())),
            )
        }
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => info("tiff", "image/tiff", None),
        [0x01, 0x00, 0x00, 0x00, ..] if data.len() > 44 && &data[40..44] == b" EMF" => {
            info("emf", "image/x-emf", None)
        }
        [0xD7, 0xCD, 0xC6, 0x9A, ..] => info("wmf", "image/x-wmf", None),
        _ => {
            return Err(Error::InvalidArgument(
                "unsupported image format; use PNG, JPEG, GIF, BMP, TIFF, EMF or WMF".into(),
            ));
        }
    })
}

/// Reads the size from a JPEG's start-of-frame marker.
fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = u32::from(u16::from_be_bytes([data[i + 5], data[i + 6]]));
            let w = u32::from(u16::from_be_bytes([data[i + 7], data[i + 8]]));
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

fn is_blip_fill(e: &Element) -> bool {
    e.is(ns::P, "blipFill") || e.is(ns::A, "blipFill")
}

fn has_blip_fill(shape: &Element) -> bool {
    let mut found = false;
    shape.walk(&mut |e| found |= is_blip_fill(e));
    found
}

/// What to write into every image fill of the shape.
struct FillUpdate<'a> {
    rid: &'a str,
    a_prefix: &'a str,
    r_prefix: &'a str,
    /// Cover crop as fractions (left/right, top/bottom).
    crop: Option<(f64, f64)>,
}

fn update_fill(fill: &mut Element, u: &FillUpdate<'_>, released: &mut HashSet<String>) {
    if fill.child(ns::A, "blip").is_none() {
        // Some producers write a picture fill without an image.
        let blip = Element {
            name: format!("{}:blip", u.a_prefix),
            ns: Some(ns::A.into()),
            attrs: Vec::new(),
            children: Vec::new(),
        };
        fill.children.insert(0, Node::Element(blip));
    }
    let blip = fill.child_mut(ns::A, "blip").expect("ensured");
    rel_ids(blip, released);
    // Extensions such as an SVG original refer to the old image.
    blip.remove_children(ns::A, "extLst");
    // A linked picture becomes an embedded one.
    blip.remove_attr_ns(ns::R, "link");
    if !blip.set_attr_ns(ns::R, "embed", u.rid) {
        blip.attrs.push(Attr {
            name: format!("{}:embed", u.r_prefix),
            ns: Some(ns::R.into()),
            value: u.rid.to_string(),
        });
    }
    fill.remove_children(ns::A, "srcRect");
    if let Some((lr, tb)) = u.crop {
        let mut rect = Element::new_like(fill.child(ns::A, "blip").expect("ensured"), "srcRect");
        // Crop is given in 1/1000 of a percent of the image size.
        for (name, v) in [("l", lr), ("t", tb), ("r", lr), ("b", tb)] {
            let v = (v * 100_000.0).round() as i64;
            if v != 0 {
                rect.set_attr(name, v.to_string());
            }
        }
        let at = fill.position(ns::A, "blip").map_or(0, |i| i + 1);
        fill.children.insert(at, Node::Element(rect));
    }
}

fn frame_mut(shape: &mut Element) -> Option<(&mut Element, i64, i64, i64, i64)> {
    let xfrm = shape.path_mut(&[(ns::P, "spPr"), (ns::A, "xfrm")])?;
    let num = |e: Option<&Element>, name| {
        e.and_then(|e| e.attr(name))
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let (x, y) = (
        num(xfrm.child(ns::A, "off"), "x"),
        num(xfrm.child(ns::A, "off"), "y"),
    );
    let (cx, cy) = (
        num(xfrm.child(ns::A, "ext"), "cx"),
        num(xfrm.child(ns::A, "ext"), "cy"),
    );
    Some((xfrm, x, y, cx, cy))
}

impl Presentation {
    /// Replaces the image of a picture (or of a shape filled with a
    /// picture), keeping its position, effects and outline. The previous
    /// image is removed from the file on save if nothing else uses it.
    pub fn replace_picture(&mut self, shape: ShapeRef, data: &[u8], fit: Fit) -> Result<()> {
        let image = sniff(data)?;
        let slide_part = self.slide_part(shape.slide)?;
        let el = self.shape_element(shape)?;
        if !has_blip_fill(el) {
            return Err(Error::Unsupported(format!(
                "shape {} is a {} without a picture",
                shape.id,
                kind_of(el).as_str()
            )));
        }
        // Prefixes for attributes and elements we may have to create.
        let slide_doc = self.pkg.xml(&slide_part)?;
        let r_prefix = slide_doc.prefix_for(ns::R).unwrap_or("r").to_string();
        let a_prefix = slide_doc.prefix_for(ns::A).unwrap_or("a").to_string();

        let media = self
            .pkg
            .next_part_name("/ppt/media/image", &format!(".{}", image.extension));
        self.pkg
            .add_raw_part(&media, image.content_type, data.to_vec());
        let rid = self
            .pkg
            .rels_mut(&slide_part)
            .add(IMAGE_REL, &relative_target(&slide_part, &media));

        let size = image.size.filter(|(w, h)| *w > 0 && *h > 0);
        let mut released = HashSet::new();
        self.with_shape_mut(shape, |el| {
            let is_picture = el.is(ns::P, "pic");
            let frame = frame_mut(el).map(|(_, x, y, cx, cy)| (x, y, cx, cy));
            let crop = match (fit, size, frame) {
                (Fit::Cover, Some((w, h)), Some((_, _, cx, cy)))
                    if is_picture && cx > 0 && cy > 0 =>
                {
                    let image_ratio = f64::from(w) / f64::from(h);
                    let frame_ratio = cx as f64 / cy as f64;
                    Some(if image_ratio > frame_ratio {
                        ((1.0 - frame_ratio / image_ratio) / 2.0, 0.0)
                    } else {
                        (0.0, (1.0 - image_ratio / frame_ratio) / 2.0)
                    })
                }
                _ => None,
            };
            let update = FillUpdate {
                rid: &rid,
                a_prefix: &a_prefix,
                r_prefix: &r_prefix,
                crop,
            };
            // Every fill: pictures in mc:AlternateContent carry one per branch.
            el.walk_mut(&mut |e| {
                if is_blip_fill(e) {
                    update_fill(e, &update, &mut released);
                }
            });

            if let (Fit::Contain, Some((w, h)), Some((x, y, cx, cy))) = (fit, size, frame)
                && is_picture
            {
                let (w, h) = (f64::from(w), f64::from(h));
                let scale = (cx as f64 / w).min(cy as f64 / h);
                let (ncx, ncy) = ((w * scale).round() as i64, (h * scale).round() as i64);
                let (xfrm, ..) = frame_mut(el).expect("frame was read above");
                let off = xfrm.ensure_child("off", &[]);
                off.set_attr("x", (x + (cx - ncx) / 2).to_string());
                off.set_attr("y", (y + (cy - ncy) / 2).to_string());
                let ext = xfrm.ensure_child("ext", &["off"]);
                ext.set_attr("cx", ncx.to_string());
                ext.set_attr("cy", ncy.to_string());
            }
            Ok(())
        })?;

        let mut still_used = HashSet::new();
        rel_ids(&self.pkg.xml(&slide_part)?.root, &mut still_used);
        let rels = self.pkg.rels_mut(&slide_part);
        for id in released.difference(&still_used) {
            rels.remove(id);
        }
        self.needs_gc = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_sizes() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(sniff(&png).unwrap().size, Some((640, 480)));

        let gif = b"GIF89a\x20\x00\x10\x00rest";
        assert_eq!(sniff(gif).unwrap().size, Some((32, 16)));

        // SOI, APP0 (len 16), SOF0 with height 200, width 300.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpeg.extend([0; 14]);
        jpeg.extend([0xFF, 0xC0, 0, 17, 8, 0, 200, 1, 44, 3]);
        jpeg.extend([0; 10]);
        assert_eq!(sniff(&jpeg).unwrap().size, Some((300, 200)));

        assert!(sniff(b"<svg/>").is_err());
    }
}
