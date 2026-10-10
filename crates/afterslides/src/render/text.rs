//! Text frames: property inheritance, body properties and layout with
//! parley.
//!
//! Text formatting comes from many places. From lowest to highest
//! priority: the presentation's default text style, the master's text
//! styles (title, body, other), the master and layout placeholders' list
//! styles, the shape's style reference (`p:style/a:fontRef`), the shape's
//! own list style, the paragraph and finally the run.

use std::borrow::Cow;

use kurbo::{Affine, Rect};
use parley::{
    Alignment, AlignmentOptions, FontFamily, FontFamilyName, FontStyle, FontWeight, GenericFamily,
    IndentOptions, LineHeight, StyleProperty,
};

use super::shapes::{Xfrm, placeholder, sp_pr};

use super::color::{ColorContext, child_color, parse_fraction};
use super::display::{Glyph, GlyphRun, Item, LineCap, LineJoin, Paint, Rgba, Stroke};
use super::scene::{EMU_PER_PT, Scene};
use crate::xml::{Element, ns};

/// Character properties; `None` means "inherit".
#[derive(Debug, Clone, Default)]
struct RunProps {
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    font: Option<String>,
    color: Option<Rgba>,
    caps: Option<bool>,
    /// Superscript/subscript offset as a fraction of the size.
    baseline: Option<f64>,
}

impl RunProps {
    fn overlay(&mut self, top: &RunProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if top.$f.is_some() { self.$f = top.$f.clone(); } )* };
        }
        take!(
            size, bold, italic, underline, strike, font, color, caps, baseline
        );
    }

    fn read(r: &Element, ctx: &ColorContext<'_>, theme: &super::color::Theme) -> RunProps {
        let flag = |n: &str| r.attr(n).map(|v| matches!(v, "1" | "true"));
        RunProps {
            size: r
                .attr("sz")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 100.0),
            bold: flag("b"),
            italic: flag("i"),
            underline: r.attr("u").map(|v| v != "none"),
            strike: r.attr("strike").map(|v| v != "noStrike"),
            font: r
                .child(ns::A, "latin")
                .and_then(|l| l.attr("typeface"))
                .map(|t| theme.font(t).to_string())
                .filter(|t| !t.is_empty()),
            color: r
                .child(ns::A, "solidFill")
                .and_then(|f| child_color(f, ctx)),
            caps: r.attr("cap").map(|v| v == "all"),
            baseline: r.attr("baseline").map(parse_fraction),
        }
    }
}

#[derive(Debug, Clone)]
enum Spacing {
    Percent(f64),
    Points(f64),
}

#[derive(Debug, Clone)]
enum Bullet {
    None,
    Char {
        ch: String,
        font: Option<String>,
        color: Option<Rgba>,
        size: Option<f64>,
    },
    Number {
        scheme: String,
        start: u32,
        font: Option<String>,
        color: Option<Rgba>,
        size: Option<f64>,
    },
}

/// Paragraph properties; `None` means "inherit".
#[derive(Debug, Clone, Default)]
struct ParaProps {
    align: Option<String>,
    mar_l: Option<f64>,
    indent: Option<f64>,
    line_spacing: Option<Spacing>,
    before: Option<Spacing>,
    after: Option<Spacing>,
    bullet: Option<Bullet>,
    run: RunProps,
}

fn spacing(el: Option<&Element>) -> Option<Spacing> {
    let el = el?;
    if let Some(p) = el.child(ns::A, "spcPct") {
        return Some(Spacing::Percent(p.attr("val").map_or(1.0, parse_fraction)));
    }
    el.child(ns::A, "spcPts")
        .and_then(|p| p.attr("val"))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| Spacing::Points(v / 100.0))
}

impl ParaProps {
    fn overlay(&mut self, top: &ParaProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if top.$f.is_some() { self.$f = top.$f.clone(); } )* };
        }
        take!(align, mar_l, indent, line_spacing, before, after, bullet);
        self.run.overlay(&top.run);
    }

    fn read(p: &Element, ctx: &ColorContext<'_>, theme: &super::color::Theme) -> ParaProps {
        let emu = |n: &str| {
            p.attr(n)
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / EMU_PER_PT)
        };
        let bu_font = p
            .child(ns::A, "buFont")
            .and_then(|f| f.attr("typeface"))
            .map(|t| theme.font(t).to_string());
        let bu_color = p.child(ns::A, "buClr").and_then(|c| child_color(c, ctx));
        let bu_size = p
            .child(ns::A, "buSzPct")
            .and_then(|s| s.attr("val"))
            .map(parse_fraction);
        let bullet = if p.child(ns::A, "buNone").is_some() {
            Some(Bullet::None)
        } else if let Some(c) = p.child(ns::A, "buChar") {
            Some(Bullet::Char {
                ch: c.attr("char").unwrap_or("•").to_string(),
                font: bu_font,
                color: bu_color,
                size: bu_size,
            })
        } else {
            p.child(ns::A, "buAutoNum").map(|n| Bullet::Number {
                scheme: n.attr("type").unwrap_or("arabicPeriod").to_string(),
                start: n.attr("startAt").and_then(|v| v.parse().ok()).unwrap_or(1),
                font: bu_font,
                color: bu_color,
                size: bu_size,
            })
        };
        ParaProps {
            align: p.attr("algn").map(str::to_string),
            mar_l: emu("marL"),
            indent: emu("indent"),
            line_spacing: spacing(p.child(ns::A, "lnSpc")),
            before: spacing(p.child(ns::A, "spcBef")),
            after: spacing(p.child(ns::A, "spcAft")),
            bullet,
            run: p
                .child(ns::A, "defRPr")
                .map(|r| RunProps::read(r, ctx, theme))
                .unwrap_or_default(),
        }
    }
}

/// Where a text body takes inherited formatting from.
pub(super) struct TextSources<'a> {
    /// Presentation default and master text style (`p:titleStyle`, ...),
    /// lowest priority first. Their `a:lvlNpPr` children give per-level
    /// defaults.
    pub master_styles: Vec<&'a Element>,
    /// Font and colour from the shape's `p:style/a:fontRef`, applied above
    /// the master styles.
    pub font_ref: Option<&'a Element>,
    /// List styles of master placeholder, layout placeholder and the shape
    /// itself, lowest priority first.
    pub shape_styles: Vec<&'a Element>,
    /// `a:bodyPr` elements, nearest first.
    pub body_props: Vec<&'a Element>,
    /// Insets (left, top, right, bottom) in points, replacing `bodyPr`'s.
    pub insets: Option<[f64; 4]>,
    /// Vertical anchor (`t`, `ctr`, `b`), replacing `bodyPr`'s.
    pub anchor: Option<String>,
    /// Bold and colour from a table style, above the master styles.
    pub style_run: Option<(Option<bool>, Option<Rgba>)>,
}

fn level_props(
    style: &Element,
    level: usize,
    ctx: &ColorContext<'_>,
    theme: &super::color::Theme,
) -> Option<ParaProps> {
    let lvl = style.child(ns::A, &format!("lvl{}pPr", level + 1))?;
    Some(ParaProps::read(lvl, ctx, theme))
}

fn roman(mut n: u32) -> String {
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (v, s) in table {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

fn number_label(scheme: &str, n: u32) -> String {
    let alpha = |n: u32| {
        let c = char::from(b'a' + ((n.max(1) - 1) % 26) as u8);
        c.to_string()
    };
    let (body, style) = if let Some(s) = scheme.strip_prefix("arabic") {
        (n.to_string(), s)
    } else if let Some(s) = scheme.strip_prefix("alphaLc") {
        (alpha(n), s)
    } else if let Some(s) = scheme.strip_prefix("alphaUc") {
        (alpha(n).to_uppercase(), s)
    } else if let Some(s) = scheme.strip_prefix("romanLc") {
        (roman(n), s)
    } else if let Some(s) = scheme.strip_prefix("romanUc") {
        (roman(n).to_uppercase(), s)
    } else {
        (n.to_string(), "Period")
    };
    match style {
        "ParenR" => format!("{body})"),
        "ParenBoth" => format!("({body})"),
        "Plain" => body,
        _ => format!("{body}."),
    }
}

/// One laid-out paragraph, positioned relative to the text area.
struct LaidOut {
    layout: parley::Layout<Rgba>,
    text: String,
    /// Underline/strike per text range.
    decorations: Vec<(std::ops::Range<usize>, bool, bool)>,
    before: f64,
    after: f64,
    x: f64,
    bullet: Option<(parley::Layout<Rgba>, String, f64)>,
    empty_height: f64,
}

impl<'a> Scene<'a, '_> {
    fn substitute(&self, family: &str) -> String {
        let subs = &self.renderer.options.substitutions;
        subs.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(family))
            .map_or_else(|| family.to_string(), |(_, v)| v.clone())
    }

    /// Lays out and draws a text body inside `rect` (shape coordinates).
    /// Returns the height the text needs, including insets.
    pub(super) fn draw_text_body(
        &mut self,
        body: &'a Element,
        sources: &TextSources<'a>,
        rect: Rect,
        transform: Affine,
    ) -> f64 {
        // Owned copies, so the scene can be borrowed mutably while laying out.
        let theme = self.layers.theme.clone();
        let color_map = self.layers.color_map.clone();
        let colors = ColorContext {
            theme: &theme,
            map: &color_map,
            placeholder: None,
        };

        // Body properties: nearest definition of each attribute wins.
        let body_attr = |name: &str| {
            sources
                .body_props
                .iter()
                .find_map(|b| b.attr(name))
                .map(str::to_string)
        };
        let inset = |name: &str, default: f64| {
            body_attr(name)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(default)
                / EMU_PER_PT
        };
        let (l, t, r, b) = match sources.insets {
            Some([l, t, r, b]) => (l, t, r, b),
            None => (
                inset("lIns", 91440.0),
                inset("tIns", 45720.0),
                inset("rIns", 91440.0),
                inset("bIns", 45720.0),
            ),
        };
        let area = Rect::new(rect.x0 + l, rect.y0 + t, rect.x1 - r, rect.y1 - b);
        let wrap = body_attr("wrap").as_deref() != Some("none");
        let anchor = sources
            .anchor
            .clone()
            .or_else(|| body_attr("anchor"))
            .unwrap_or_else(|| "t".into());
        let autofit = sources
            .body_props
            .iter()
            .find_map(|b| b.child(ns::A, "normAutofit"));
        let font_scale = autofit
            .and_then(|a| a.attr("fontScale"))
            .map_or(1.0, parse_fraction);
        let spacing_reduction = autofit
            .and_then(|a| a.attr("lnSpcReduction"))
            .map_or(0.0, parse_fraction);

        // Defaults from the shape's font reference.
        let mut base_run = RunProps {
            size: Some(18.0),
            font: Some(theme.minor_font.clone()),
            color: Some(Rgba::BLACK),
            ..RunProps::default()
        };
        let mut paragraphs = Vec::new();
        let mut counters = [0u32; 9];
        let paras: Vec<&Element> = body.children_named(ns::A, "p").collect();
        for (pi, p) in paras.iter().enumerate() {
            let ppr = p.child(ns::A, "pPr");
            let level = ppr
                .and_then(|x| x.attr("lvl"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
                .min(8);
            let mut props = ParaProps {
                run: base_run.clone(),
                ..ParaProps::default()
            };
            for style in &sources.master_styles {
                if let Some(lp) = level_props(style, level, &colors, &theme) {
                    props.overlay(&lp);
                }
            }
            if let Some(fr) = sources.font_ref {
                let mut from_ref = RunProps::default();
                match fr.attr("idx") {
                    Some("major") => from_ref.font = Some(theme.major_font.clone()),
                    Some("minor") => from_ref.font = Some(theme.minor_font.clone()),
                    _ => {}
                }
                from_ref.color = child_color(fr, &colors);
                props.run.overlay(&from_ref);
            }
            if let Some((bold, color)) = sources.style_run {
                let from_style = RunProps {
                    bold,
                    color,
                    ..RunProps::default()
                };
                props.run.overlay(&from_style);
            }
            for style in &sources.shape_styles {
                if let Some(lp) = level_props(style, level, &colors, &theme) {
                    props.overlay(&lp);
                }
            }
            if let Some(ppr) = ppr {
                props.overlay(&ParaProps::read(ppr, &colors, &theme));
            }
            if pi == 0 {
                base_run = props.run.clone();
            }

            // Numbering restarts whenever a paragraph at this level has none.
            let numbered = matches!(props.bullet, Some(Bullet::Number { .. }));
            for (lvl, c) in counters.iter_mut().enumerate() {
                if lvl > level || (lvl == level && !numbered) {
                    *c = 0;
                }
            }

            let laid = self.layout_paragraph(
                p,
                &props,
                level,
                &mut counters,
                font_scale,
                spacing_reduction,
                wrap,
                area.width(),
                &colors,
                &theme,
            );
            paragraphs.push(laid);
        }

        // Vertical placement.
        let total = text_height(&paragraphs);
        let mut y = match anchor.as_str() {
            "ctr" => area.y0 + (area.height() - total) / 2.0,
            "b" => area.y1 - total,
            _ => area.y0,
        };
        for (i, p) in paragraphs.iter().enumerate() {
            if i > 0 {
                y += p.before;
            }
            self.emit_paragraph(p, area.x0 + p.x, y, transform);
            y += f64::from(p.layout.height()).max(p.empty_height) + p.after;
        }
        total + t + b
    }
}

fn text_height(paragraphs: &[LaidOut]) -> f64 {
    let mut total = 0.0;
    for (i, p) in paragraphs.iter().enumerate() {
        if i > 0 {
            total += p.before;
        }
        total += f64::from(p.layout.height()).max(p.empty_height);
        if i + 1 < paragraphs.len() {
            total += p.after;
        }
    }
    total
}

impl<'a> Scene<'a, '_> {
    #[allow(clippy::too_many_arguments)]
    fn layout_paragraph(
        &mut self,
        p: &Element,
        props: &ParaProps,
        level: usize,
        counters: &mut [u32; 9],
        font_scale: f64,
        spacing_reduction: f64,
        wrap: bool,
        width: f64,
        colors: &ColorContext<'_>,
        theme: &super::color::Theme,
    ) -> LaidOut {
        // Collect text and per-range run properties.
        let mut text = String::new();
        let mut spans: Vec<(std::ops::Range<usize>, RunProps)> = Vec::new();
        let mut first_run: Option<RunProps> = None;
        for child in p.elements() {
            let (content, rpr) = match child.local() {
                "r" => (
                    child
                        .child(ns::A, "t")
                        .map(Element::text)
                        .unwrap_or_default(),
                    child.child(ns::A, "rPr"),
                ),
                "br" => ("\n".to_string(), child.child(ns::A, "rPr")),
                "fld" => {
                    let mut t = child
                        .child(ns::A, "t")
                        .map(Element::text)
                        .unwrap_or_default();
                    if child.attr("type") == Some("slidenum") {
                        t = self.slide_number.to_string();
                    }
                    (t, child.child(ns::A, "rPr"))
                }
                _ => continue,
            };
            let mut run = props.run.clone();
            if let Some(rpr) = rpr {
                run.overlay(&RunProps::read(rpr, colors, theme));
                if rpr.child(ns::A, "hlinkClick").is_some() {
                    if let Some(c) = theme.color("hlink") {
                        run.color = Some(c);
                    }
                    run.underline = Some(true);
                }
            }
            let content = if run.caps == Some(true) {
                content.to_uppercase()
            } else {
                content
            };
            let start = text.len();
            text.push_str(&content);
            if first_run.is_none() {
                first_run = Some(run.clone());
            }
            spans.push((start..text.len(), run));
        }
        let mut end_props = props.run.clone();
        if let Some(end) = p.child(ns::A, "endParaRPr") {
            end_props.overlay(&RunProps::read(end, colors, theme));
        }
        let first = first_run.unwrap_or_else(|| end_props.clone());
        let first_size = first.size.unwrap_or(18.0) * font_scale;

        let line_height = match &props.line_spacing {
            Some(Spacing::Points(pt)) => LineHeight::Absolute((pt * font_scale) as f32),
            Some(Spacing::Percent(pct)) => {
                LineHeight::MetricsRelative((pct * (1.0 - spacing_reduction)) as f32)
            }
            None => LineHeight::MetricsRelative((1.0 - spacing_reduction) as f32),
        };
        let resolve_space = |s: &Option<Spacing>| match s {
            Some(Spacing::Points(pt)) => *pt,
            Some(Spacing::Percent(pct)) => pct * first_size * 1.2,
            None => 0.0,
        };

        let build = |scene: &mut Self, text: &str, spans: &[(std::ops::Range<usize>, RunProps)]| {
            let families: Vec<String> = spans
                .iter()
                .map(|(_, run)| scene.substitute(run.font.as_deref().unwrap_or("Calibri")))
                .collect();
            let renderer = &mut *scene.renderer;
            let (fcx, lcx) = renderer.contexts();
            let mut builder = lcx.ranged_builder(fcx, text, 1.0, false);
            builder.push_default(StyleProperty::LineHeight(line_height));
            builder.push_default(StyleProperty::FontSize(first_size as f32));
            for ((range, run), family) in spans.iter().zip(families) {
                let families: Vec<FontFamilyName<'static>> = vec![
                    FontFamilyName::Named(Cow::Owned(family)),
                    FontFamilyName::Generic(GenericFamily::SansSerif),
                ];
                builder.push(
                    StyleProperty::FontFamily(FontFamily::List(Cow::Owned(families))),
                    range.clone(),
                );
                builder.push(
                    StyleProperty::FontSize(
                        (run.size.unwrap_or(18.0)
                            * font_scale
                            * if run.baseline.is_some_and(|b| b != 0.0) {
                                0.66
                            } else {
                                1.0
                            }) as f32,
                    ),
                    range.clone(),
                );
                if run.bold == Some(true) {
                    builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
                }
                if run.italic == Some(true) {
                    builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
                }
                builder.push(
                    StyleProperty::Brush(run.color.unwrap_or(Rgba::BLACK)),
                    range.clone(),
                );
            }
            builder.build(text)
        };

        let mar_l = props.mar_l.unwrap_or(0.0);
        let indent = props.indent.unwrap_or(0.0);
        let has_text = !text.trim().is_empty();
        let bullet_text = match (&props.bullet, has_text) {
            (Some(Bullet::Char { ch, .. }), true) => Some(ch.clone()),
            (Some(Bullet::Number { scheme, start, .. }), true) => {
                counters[level] += 1;
                Some(number_label(scheme, start + counters[level] - 1))
            }
            _ => None,
        };

        let max_width = (width - mar_l).max(1.0);
        let mut layout = build(self, &text, &spans);
        if bullet_text.is_none() && indent != 0.0 {
            layout.set_text_indent(indent as f32, IndentOptions::default());
        }
        layout.break_all_lines(wrap.then_some(max_width as f32));
        let alignment = match props.align.as_deref() {
            Some("ctr") => Alignment::Center,
            Some("r") => Alignment::Right,
            Some("just" | "dist" | "justLow") => Alignment::Justify,
            _ => Alignment::Left,
        };
        layout.align(alignment, AlignmentOptions::default());

        let bullet = bullet_text.map(|label| {
            let (font, color, size) = match &props.bullet {
                Some(
                    Bullet::Char {
                        font, color, size, ..
                    }
                    | Bullet::Number {
                        font, color, size, ..
                    },
                ) => (font.clone(), *color, *size),
                _ => (None, None, None),
            };
            let run = RunProps {
                font: font.or_else(|| first.font.clone()),
                color: color.or(first.color),
                size: Some(first.size.unwrap_or(18.0) * size.unwrap_or(1.0)),
                ..RunProps::default()
            };
            let mut bl = build(self, &label, &[(0..label.len(), run)]);
            bl.break_all_lines(None);
            bl.align(Alignment::Left, AlignmentOptions::default());
            (bl, label, indent)
        });

        let decorations = spans
            .iter()
            .filter(|(_, r)| r.underline == Some(true) || r.strike == Some(true))
            .map(|(range, r)| {
                (
                    range.clone(),
                    r.underline == Some(true),
                    r.strike == Some(true),
                )
            })
            .collect();
        let empty_height = if has_text {
            0.0
        } else {
            end_props.size.unwrap_or(18.0) * font_scale * 1.2
        };
        LaidOut {
            layout,
            text,
            decorations,
            before: resolve_space(&props.before),
            after: resolve_space(&props.after),
            x: mar_l,
            bullet,
            empty_height,
        }
    }

    fn emit_paragraph(&mut self, p: &LaidOut, x: f64, y: f64, transform: Affine) {
        let at = transform * Affine::translate((x, y));
        emit_layout(
            &mut self.items,
            &p.layout,
            &p.text,
            at,
            &p.decorations,
            &mut self.renderer.font_ids,
        );
        if let (Some((bullet, label, indent)), Some(first_line)) =
            (&p.bullet, p.layout.lines().next())
        {
            // Align the bullet's baseline with the first text line.
            let text_baseline = f64::from(first_line.metrics().baseline);
            let bullet_baseline = bullet
                .lines()
                .next()
                .map_or(0.0, |l| f64::from(l.metrics().baseline));
            let at =
                transform * Affine::translate((x + indent, y + text_baseline - bullet_baseline));
            emit_layout(
                &mut self.items,
                bullet,
                label,
                at,
                &[],
                &mut self.renderer.font_ids,
            );
        }
    }
}

/// Converts a parley layout into glyph runs (and decoration lines).
fn emit_layout(
    items: &mut Vec<Item>,
    layout: &parley::Layout<Rgba>,
    text: &str,
    transform: Affine,
    decorations: &[(std::ops::Range<usize>, bool, bool)],
    font_ids: &mut super::FontIds,
) {
    for line in layout.lines() {
        let baseline = f64::from(line.metrics().baseline);
        let mut x = f64::from(line.metrics().offset);
        for run in line.runs() {
            let font = run.font();
            let font_ref = font_ids.get(font);
            let size = f64::from(run.font_size());
            let metrics = run.font_metrics();
            let mut current: Option<GlyphRun> = None;
            let flush = |items: &mut Vec<Item>, run: Option<GlyphRun>| {
                if let Some(run) = run.filter(|r| !r.glyphs.is_empty()) {
                    items.push(Item::Glyphs { run, transform });
                }
            };
            for cluster in run.visual_clusters() {
                let color = cluster.style().brush;
                let range = cluster.text_range();
                // A ligature's later characters have no glyph of their own;
                // their text belongs to the ligature glyph.
                if cluster.is_ligature_continuation() {
                    if let Some(last) = current.as_mut().and_then(|c| c.clusters.last_mut()) {
                        last.end = last.end.max(range.end);
                    }
                    continue;
                }
                let start_x = x;
                for (gi, g) in cluster.glyphs().enumerate() {
                    if current.as_ref().is_some_and(|c| c.color != color) {
                        flush(items, current.take());
                    }
                    let cur = current.get_or_insert_with(|| GlyphRun {
                        font: font_ref.clone(),
                        size,
                        color,
                        glyphs: Vec::new(),
                        text: text.to_string(),
                        clusters: Vec::new(),
                    });
                    cur.glyphs.push(Glyph {
                        id: g.id,
                        x: x + f64::from(g.x),
                        y: baseline - f64::from(g.y),
                        advance: f64::from(g.advance),
                    });
                    // The first glyph of a cluster carries its text.
                    cur.clusters.push(if gi == 0 {
                        range.clone()
                    } else {
                        range.end..range.end
                    });
                    x += f64::from(g.advance);
                }
                // Underline / strikethrough for decorated text.
                if let Some((_, under, strike)) = decorations
                    .iter()
                    .find(|(r, _, _)| r.start <= range.start && range.end <= r.end)
                    && x > start_x
                {
                    let mut lines = Vec::new();
                    if *under {
                        let off = -f64::from(metrics.underline_offset);
                        let w = f64::from(metrics.underline_size).max(0.5);
                        lines.push((baseline + off, w));
                    }
                    if *strike {
                        let off = -f64::from(metrics.strikethrough_offset);
                        let w = f64::from(metrics.strikethrough_size).max(0.5);
                        lines.push((baseline + off, w));
                    }
                    for (ly, w) in lines {
                        let mut path = kurbo::BezPath::new();
                        path.move_to((start_x, ly));
                        path.line_to((x, ly));
                        items.push(Item::Stroke {
                            path,
                            stroke: Stroke {
                                paint: Paint::Solid(color),
                                width: w,
                                cap: LineCap::Butt,
                                join: LineJoin::Miter,
                                dash: Vec::new(),
                            },
                            transform,
                        });
                    }
                }
            }
            flush(items, current.take());
        }
    }
}

impl<'a> Scene<'a, '_> {
    /// Text of a shape (`p:txBody`), formatted through its inheritance chain.
    pub(super) fn text_frame(
        &mut self,
        chain: &[&'a Element],
        rect: Rect,
        transform: Affine,
        _xfrm: Xfrm,
    ) {
        let Some(body) = chain[0].child(ns::P, "txBody") else {
            return;
        };
        let prs_root = &self
            .layers
            .prs
            .pkg
            .xml(&self.layers.prs.main)
            .map(|d| &d.root)
            .ok();
        let mut master_styles = Vec::new();
        if let Some(Some(root)) = prs_root.map(|r| r.child(ns::P, "defaultTextStyle")) {
            master_styles.push(root);
        }
        if let (Some(ph), Some(master)) = (placeholder(chain[0]), self.layers.master) {
            let style = match ph.kind.as_str() {
                "title" | "ctrTitle" => "titleStyle",
                "dt" | "ftr" | "sldNum" | "hdr" => "otherStyle",
                _ => "bodyStyle",
            };
            if let Some(s) = master.root.path(&[(ns::P, "txStyles"), (ns::P, style)]) {
                master_styles.push(s);
            }
        }
        let shape_styles = chain
            .iter()
            .rev()
            .filter_map(|el| el.path(&[(ns::P, "txBody"), (ns::A, "lstStyle")]))
            .collect();
        let body_props = chain
            .iter()
            .filter_map(|el| el.path(&[(ns::P, "txBody"), (ns::A, "bodyPr")]))
            .collect();
        let sources = TextSources {
            master_styles,
            font_ref: chain[0].path(&[(ns::P, "style"), (ns::A, "fontRef")]),
            shape_styles,
            body_props,
            insets: None,
            anchor: None,
            style_run: None,
        };
        let _ = sp_pr;
        self.draw_text_body(body, &sources, rect, transform);
    }
}

/// Horizontal anchor of a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HAlign {
    Left,
    Center,
    Right,
}

/// A single line of text with one style, e.g. a chart label.
pub(super) struct Label<'t> {
    pub text: &'t str,
    pub size: f64,
    pub bold: bool,
    pub color: Rgba,
    pub font: Option<&'t str>,
}

impl Scene<'_, '_> {
    /// Lays out a label; returns its size (width, height) and the layout.
    fn label_layout(&mut self, label: &Label<'_>) -> (f64, f64, parley::Layout<Rgba>) {
        let family = self.substitute(label.font.unwrap_or(&self.layers.theme.minor_font.clone()));
        let (fcx, lcx) = self.renderer.contexts();
        let mut builder = lcx.ranged_builder(fcx, label.text, 1.0, false);
        let families = vec![
            FontFamilyName::Named(Cow::Owned(family)),
            FontFamilyName::Generic(GenericFamily::SansSerif),
        ];
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(
            families,
        ))));
        builder.push_default(StyleProperty::FontSize(label.size as f32));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        if label.bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        builder.push_default(StyleProperty::Brush(label.color));
        let mut layout = builder.build(label.text);
        layout.break_all_lines(None);
        layout.align(Alignment::Left, AlignmentOptions::default());
        (
            f64::from(layout.width()),
            f64::from(layout.height()),
            layout,
        )
    }

    pub(super) fn measure_label(&mut self, label: &Label<'_>) -> (f64, f64) {
        let (w, h, _) = self.label_layout(label);
        (w, h)
    }

    /// Draws a label with its top edge at `y` and `x` given by `align`.
    pub(super) fn draw_label(
        &mut self,
        label: &Label<'_>,
        x: f64,
        y: f64,
        align: HAlign,
        transform: Affine,
    ) -> (f64, f64) {
        let (w, h, layout) = self.label_layout(label);
        let left = match align {
            HAlign::Left => x,
            HAlign::Center => x - w / 2.0,
            HAlign::Right => x - w,
        };
        emit_layout(
            &mut self.items,
            &layout,
            label.text,
            transform * Affine::translate((left, y)),
            &[],
            &mut self.renderer.font_ids,
        );
        (w, h)
    }
}
