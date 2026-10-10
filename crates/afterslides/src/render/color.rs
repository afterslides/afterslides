//! Themes and DrawingML colours.

use std::collections::HashMap;

use super::display::Rgba;
use crate::xml::{Element, ns};

/// What a slide's theme contributes to rendering.
#[derive(Debug, Clone, Default)]
pub struct Theme {
    colors: HashMap<String, Rgba>,
    pub major_font: String,
    pub minor_font: String,
    pub fill_styles: Vec<Element>,
    pub line_styles: Vec<Element>,
    pub bg_fill_styles: Vec<Element>,
}

impl Theme {
    pub fn parse(root: &Element) -> Theme {
        let mut theme = Theme::default();
        let Some(elements) = root.child(ns::A, "themeElements") else {
            return theme;
        };
        if let Some(scheme) = elements.child(ns::A, "clrScheme") {
            for slot in scheme.elements() {
                if let Some(color) = slot.elements().find_map(|c| base_color(c, None, None)) {
                    theme.colors.insert(slot.local().to_string(), color);
                }
            }
        }
        if let Some(fonts) = elements.child(ns::A, "fontScheme") {
            let latin = |kind: &str| {
                fonts
                    .path(&[(ns::A, kind), (ns::A, "latin")])
                    .and_then(|l| l.attr("typeface"))
                    .unwrap_or_default()
                    .to_string()
            };
            theme.major_font = latin("majorFont");
            theme.minor_font = latin("minorFont");
        }
        if let Some(fmt) = elements.child(ns::A, "fmtScheme") {
            let list = |name: &str| {
                fmt.child(ns::A, name)
                    .map(|l| l.elements().cloned().collect())
                    .unwrap_or_default()
            };
            theme.fill_styles = list("fillStyleLst");
            theme.line_styles = list("lnStyleLst");
            theme.bg_fill_styles = list("bgFillStyleLst");
        }
        theme
    }

    pub fn color(&self, slot: &str) -> Option<Rgba> {
        self.colors.get(slot).copied()
    }

    /// `+mj-lt` / `+mn-lt` and friends resolve to the theme fonts.
    pub fn font<'a>(&'a self, typeface: &'a str) -> &'a str {
        match typeface {
            t if t.starts_with("+mj") => &self.major_font,
            t if t.starts_with("+mn") => &self.minor_font,
            t => t,
        }
    }
}

/// Everything needed to turn a colour element into RGB.
#[derive(Debug, Clone, Copy)]
pub struct ColorContext<'a> {
    pub theme: &'a Theme,
    /// The master's `p:clrMap` (bg1 -> lt1, tx1 -> dk1, ...), possibly
    /// overridden by the slide.
    pub map: &'a HashMap<String, String>,
    /// The colour `phClr` stands for when resolving theme style references.
    pub placeholder: Option<Rgba>,
}

impl ColorContext<'_> {
    pub fn with_placeholder(&self, color: Option<Rgba>) -> ColorContext<'_> {
        ColorContext {
            placeholder: color,
            ..*self
        }
    }
}

/// Reads a `p:clrMap` into slot -> theme colour name.
pub fn parse_color_map(el: &Element) -> HashMap<String, String> {
    el.attrs
        .iter()
        .filter(|a| a.ns.is_none())
        .map(|a| (a.name.clone(), a.value.clone()))
        .collect()
}

pub fn default_color_map() -> HashMap<String, String> {
    [
        ("bg1", "lt1"),
        ("tx1", "dk1"),
        ("bg2", "lt2"),
        ("tx2", "dk2"),
        ("accent1", "accent1"),
        ("accent2", "accent2"),
        ("accent3", "accent3"),
        ("accent4", "accent4"),
        ("accent5", "accent5"),
        ("accent6", "accent6"),
        ("hlink", "hlink"),
        ("folHlink", "folHlink"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

fn is_color_element(e: &Element) -> bool {
    e.ns() == Some(ns::A)
        && matches!(
            e.local(),
            "srgbClr" | "schemeClr" | "sysClr" | "prstClr" | "scrgbClr" | "hslClr"
        )
}

/// The colour of the first colour element among `parent`'s children.
pub fn child_color(parent: &Element, ctx: &ColorContext<'_>) -> Option<Rgba> {
    parent
        .elements()
        .find(|e| is_color_element(e))
        .and_then(|c| color(c, ctx))
}

/// Resolves a colour element, including its modifiers.
pub fn color(el: &Element, ctx: &ColorContext<'_>) -> Option<Rgba> {
    let base = base_color(el, Some(ctx), ctx.placeholder)?;
    Some(apply_modifiers(base, el))
}

fn base_color(
    el: &Element,
    ctx: Option<&ColorContext<'_>>,
    placeholder: Option<Rgba>,
) -> Option<Rgba> {
    if el.ns() != Some(ns::A) {
        return None;
    }
    match el.local() {
        "srgbClr" => parse_hex(el.attr("val")?),
        "sysClr" => el
            .attr("lastClr")
            .and_then(parse_hex)
            .or_else(|| match el.attr("val")? {
                "window" => Some(Rgba::WHITE),
                _ => Some(Rgba::BLACK),
            }),
        "schemeClr" => {
            let val = el.attr("val")?;
            if val == "phClr" {
                return placeholder;
            }
            let ctx = ctx?;
            let slot = ctx.map.get(val).map_or(val, String::as_str);
            ctx.theme.color(slot)
        }
        "prstClr" => preset_color(el.attr("val")?),
        "scrgbClr" => {
            let c = |n| el.attr(n).map_or(0.0, parse_fraction);
            Some(from_linear(c("r"), c("g"), c("b")))
        }
        "hslClr" => {
            let hue = el
                .attr("hue")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 60000.0;
            let sat = el.attr("sat").map_or(0.0, parse_fraction);
            let lum = el.attr("lum").map_or(0.0, parse_fraction);
            let (r, g, b) = hsl_to_rgb(hue, sat, lum);
            Some(from_unit(r, g, b, 1.0))
        }
        _ => None,
    }
}

fn parse_hex(v: &str) -> Option<Rgba> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some(Rgba::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// "50000" (thousandths of a percent) or "50%" -> 0.5.
pub fn parse_fraction(v: &str) -> f64 {
    let v = v.trim();
    match v.strip_suffix('%') {
        Some(p) => p.parse::<f64>().unwrap_or(0.0) / 100.0,
        None => v.parse::<f64>().unwrap_or(0.0) / 100_000.0,
    }
}

fn preset_color(name: &str) -> Option<Rgba> {
    let hex = match name {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "gray" | "grey" => "808080",
        "silver" => "C0C0C0",
        "lightGray" | "lightGrey" | "ltGray" => "D3D3D3",
        "darkGray" | "darkGrey" | "dkGray" => "A9A9A9",
        "orange" => "FFA500",
        "navy" => "000080",
        "maroon" => "800000",
        "purple" => "800080",
        "teal" => "008080",
        "olive" => "808000",
        _ => return Some(Rgba::BLACK),
    };
    parse_hex(hex)
}

// ---- colour maths ---------------------------------------------------------

fn to_unit(c: Rgba) -> (f64, f64, f64) {
    (
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
    )
}

fn from_unit(r: f64, g: f64, b: f64, a: f64) -> Rgba {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba {
        r: q(r),
        g: q(g),
        b: q(b),
        a: a.clamp(0.0, 1.0) as f32,
    }
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn from_linear(r: f64, g: f64, b: f64) -> Rgba {
    from_unit(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b), 1.0)
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let h = h / 360.0;
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

/// Applies `lumMod`, `tint`, `alpha`, ... in document order.
fn apply_modifiers(base: Rgba, el: &Element) -> Rgba {
    let (mut r, mut g, mut b) = to_unit(base);
    let mut a = f64::from(base.a);
    for m in el.elements().filter(|m| m.ns() == Some(ns::A)) {
        let v = m.attr("val").map_or(0.0, parse_fraction);
        match m.local() {
            "alpha" => a = v,
            "alphaMod" => a *= v,
            "alphaOff" => a += v,
            "lumMod" | "lumOff" | "lum" | "satMod" | "satOff" | "sat" | "hueMod" | "hueOff"
            | "hue" => {
                let (mut h, mut s, mut l) = rgb_to_hsl(r, g, b);
                let deg = m
                    .attr("val")
                    .and_then(|x| x.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 60000.0;
                match m.local() {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    "lum" => l = v,
                    "satMod" => s *= v,
                    "satOff" => s += v,
                    "sat" => s = v,
                    "hueMod" => h *= v,
                    "hueOff" => h += deg,
                    "hue" => h = deg,
                    _ => {}
                }
                (r, g, b) = hsl_to_rgb(h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
            }
            // Tint and shade work on linear RGB, as Office does.
            "tint" => {
                let t = |c: f64| linear_to_srgb(1.0 - (1.0 - srgb_to_linear(c)) * v);
                (r, g, b) = (t(r), t(g), t(b));
            }
            "shade" => {
                let s = |c: f64| linear_to_srgb(srgb_to_linear(c) * v);
                (r, g, b) = (s(r), s(g), s(b));
            }
            "comp" => {
                let (h, s, l) = rgb_to_hsl(r, g, b);
                (r, g, b) = hsl_to_rgb((h + 180.0).rem_euclid(360.0), s, l);
            }
            "inv" => (r, g, b) = (1.0 - r, 1.0 - g, 1.0 - b),
            "gray" => {
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                (r, g, b) = (y, y, y);
            }
            _ => {}
        }
    }
    from_unit(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Document;

    fn eval(xml: &str) -> Rgba {
        let theme = Theme::default();
        let map = default_color_map();
        let ctx = ColorContext {
            theme: &theme,
            map: &map,
            placeholder: None,
        };
        let wrapped = format!(
            r#"<x xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{xml}</x>"#
        );
        let doc = Document::parse(wrapped.as_bytes()).unwrap();
        child_color(&doc.root, &ctx).unwrap()
    }

    #[test]
    fn modifiers() {
        assert_eq!(
            eval(r#"<a:srgbClr val="4472C4"/>"#),
            Rgba::rgb(0x44, 0x72, 0xC4)
        );
        // 75% luminance of accent1 blue, as used by PowerPoint for "darker 25%".
        let darker = eval(r#"<a:srgbClr val="4472C4"><a:lumMod val="75000"/></a:srgbClr>"#);
        assert_eq!((darker.r, darker.g, darker.b), (0x2F, 0x55, 0x97));
        let lighter = eval(
            r#"<a:srgbClr val="4472C4"><a:lumMod val="40000"/><a:lumOff val="60000"/></a:srgbClr>"#,
        );
        assert_eq!((lighter.r, lighter.g, lighter.b), (0xB4, 0xC7, 0xE7));
        let half = eval(r#"<a:srgbClr val="FF0000"><a:alpha val="50%"/></a:srgbClr>"#);
        assert!((half.a - 0.5).abs() < 1e-6);
    }
}
