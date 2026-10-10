//! DrawingML geometry: the guide formula language and path commands used
//! by both preset shapes (`a:prstGeom`) and custom shapes (`a:custGeom`).
//!
//! Preset definitions come from ECMA-376 Annex D (see NOTICE) and are parsed
//! on first use. Evaluation never panics: bad formulas evaluate to 0.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::io::Read;
use std::sync::OnceLock;

use kurbo::{Arc, BezPath, Point, Rect};

use crate::xml::{Document, Element, ns};

static PRESETS: OnceLock<HashMap<String, Element>> = OnceLock::new();

fn presets() -> &'static HashMap<String, Element> {
    PRESETS.get_or_init(|| {
        let compressed = include_bytes!("presetShapeDefinitions.xml.z");
        let mut xml = Vec::new();
        if flate2::read::ZlibDecoder::new(&compressed[..])
            .read_to_end(&mut xml)
            .is_err()
        {
            return HashMap::new();
        }
        let Ok(doc) = Document::parse(&xml) else {
            return HashMap::new();
        };
        doc.root
            .elements()
            .map(|e| (e.name.clone(), e.clone()))
            .collect()
    })
}

/// How a sub-path is filled (`path/@fill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFill {
    None,
    Normal,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

#[derive(Debug, Clone)]
pub struct SubPath {
    pub path: BezPath,
    pub fill: PathFill,
    pub stroke: bool,
}

#[derive(Debug, Clone)]
pub struct Geometry {
    pub paths: Vec<SubPath>,
    /// Where text goes, in shape coordinates.
    pub text_rect: Rect,
}

/// Geometry of a shape of size `w` x `h` (points) from its `spPr`.
pub fn shape_geometry(sp_pr: Option<&Element>, w: f64, h: f64) -> Geometry {
    let rect = || Geometry {
        paths: vec![SubPath {
            path: kurbo::Shape::to_path(&Rect::new(0.0, 0.0, w, h), 0.1),
            fill: PathFill::Normal,
            stroke: true,
        }],
        text_rect: Rect::new(0.0, 0.0, w, h),
    };
    let Some(sp_pr) = sp_pr else { return rect() };
    if let Some(cust) = sp_pr.child(ns::A, "custGeom") {
        return evaluate(cust, None, w, h);
    }
    if let Some(prst) = sp_pr.child(ns::A, "prstGeom") {
        let name = prst.attr("prst").unwrap_or("rect");
        if name == "rect" {
            return rect();
        }
        if let Some(def) = presets().get(name) {
            return evaluate(def, prst.child(ns::A, "avLst"), w, h);
        }
    }
    rect()
}

struct Guides {
    values: HashMap<String, f64>,
}

impl Guides {
    fn new(w: f64, h: f64) -> Guides {
        let ss = w.min(h);
        let mut values = HashMap::new();
        let mut set = |k: &str, v: f64| {
            values.insert(k.to_string(), v);
        };
        set("w", w);
        set("h", h);
        set("l", 0.0);
        set("t", 0.0);
        set("r", w);
        set("b", h);
        set("hc", w / 2.0);
        set("vc", h / 2.0);
        set("ss", ss);
        set("ls", w.max(h));
        for d in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 32.0] {
            set(&format!("wd{d}"), w / d);
            set(&format!("hd{d}"), h / d);
            set(&format!("ssd{d}"), ss / d);
        }
        set("cd2", 10_800_000.0);
        set("cd4", 5_400_000.0);
        set("cd8", 2_700_000.0);
        set("3cd4", 16_200_000.0);
        set("3cd8", 8_100_000.0);
        set("5cd8", 13_500_000.0);
        set("7cd8", 18_900_000.0);
        Guides { values }
    }

    fn get(&self, token: &str) -> f64 {
        match self.values.get(token) {
            Some(v) => *v,
            None => token.parse::<f64>().unwrap_or(0.0),
        }
    }

    fn eval(&self, formula: &str) -> f64 {
        let mut parts = formula.split_whitespace();
        let op = parts.next().unwrap_or("");
        let args: Vec<f64> = parts.map(|t| self.get(t)).collect();
        let a = |i: usize| args.get(i).copied().unwrap_or(0.0);
        let angle = |v: f64| v / 60000.0 * PI / 180.0;
        let result = match op {
            "val" => a(0),
            "*/" => {
                if a(2) == 0.0 {
                    0.0
                } else {
                    a(0) * a(1) / a(2)
                }
            }
            "+-" => a(0) + a(1) - a(2),
            "+/" => {
                if a(2) == 0.0 {
                    0.0
                } else {
                    (a(0) + a(1)) / a(2)
                }
            }
            "?:" => {
                if a(0) > 0.0 {
                    a(1)
                } else {
                    a(2)
                }
            }
            "abs" => a(0).abs(),
            "at2" => a(1).atan2(a(0)) * 180.0 / PI * 60000.0,
            "cat2" => a(0) * a(2).atan2(a(1)).cos(),
            "sat2" => a(0) * a(2).atan2(a(1)).sin(),
            "cos" => a(0) * angle(a(1)).cos(),
            "sin" => a(0) * angle(a(1)).sin(),
            "tan" => a(0) * angle(a(1)).tan(),
            "max" => a(0).max(a(1)),
            "min" => a(0).min(a(1)),
            "mod" => (a(0) * a(0) + a(1) * a(1) + a(2) * a(2)).sqrt(),
            "pin" => {
                if a(1) < a(0) {
                    a(0)
                } else if a(1) > a(2) {
                    a(2)
                } else {
                    a(1)
                }
            }
            "sqrt" => a(0).max(0.0).sqrt(),
            _ => 0.0,
        };
        if result.is_finite() { result } else { 0.0 }
    }

    fn load(&mut self, list: Option<&Element>) {
        let Some(list) = list else { return };
        for gd in list.children_named(ns::A, "gd") {
            if let (Some(name), Some(fmla)) = (gd.attr("name"), gd.attr("fmla")) {
                let v = self.eval(fmla);
                self.values.insert(name.to_string(), v);
            }
        }
    }
}

/// Maps the ellipse's true angle `phi` to its parametric angle, keeping
/// whole turns so sweeps of 360 degrees and more work.
fn parametric(phi: f64, rx: f64, ry: f64) -> f64 {
    let t = (rx * phi.sin()).atan2(ry * phi.cos());
    t + (2.0 * PI) * ((phi - t) / (2.0 * PI)).round()
}

fn evaluate(def: &Element, overrides: Option<&Element>, w: f64, h: f64) -> Geometry {
    let mut guides = Guides::new(w, h);
    guides.load(def.child(ns::A, "avLst"));
    guides.load(overrides);
    guides.load(def.child(ns::A, "gdLst"));

    let mut paths = Vec::new();
    if let Some(list) = def.child(ns::A, "pathLst") {
        for path in list.children_named(ns::A, "path") {
            paths.push(sub_path(path, &guides, w, h));
        }
    }
    let text_rect = def
        .child(ns::A, "rect")
        .map(|r| {
            let v = |n: &str| r.attr(n).map_or(0.0, |t| guides.get(t));
            Rect::new(v("l"), v("t"), v("r"), v("b"))
        })
        .unwrap_or(Rect::new(0.0, 0.0, w, h));
    Geometry { paths, text_rect }
}

fn sub_path(path: &Element, guides: &Guides, w: f64, h: f64) -> SubPath {
    // Path coordinates may have their own size; scale them to the shape.
    let pw = path
        .attr("w")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0);
    let ph = path
        .attr("h")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0);
    let sx = pw.map_or(1.0, |pw| w / pw);
    let sy = ph.map_or(1.0, |ph| h / ph);
    let point = |pt: &Element| {
        let x = pt.attr("x").map_or(0.0, |t| guides.get(t));
        let y = pt.attr("y").map_or(0.0, |t| guides.get(t));
        Point::new(x * sx, y * sy)
    };

    let mut out = BezPath::new();
    let mut current = Point::ZERO;
    let mut started = false;
    for cmd in path.elements() {
        let pts: Vec<Point> = cmd.children_named(ns::A, "pt").map(point).collect();
        match cmd.local() {
            "moveTo" => {
                if let Some(p) = pts.first() {
                    out.move_to(*p);
                    current = *p;
                    started = true;
                }
            }
            "lnTo" => {
                if let Some(p) = pts.first() {
                    if !started {
                        out.move_to(current);
                        started = true;
                    }
                    out.line_to(*p);
                    current = *p;
                }
            }
            "quadBezTo" => {
                if let [a, p, ..] = pts[..] {
                    if !started {
                        out.move_to(current);
                        started = true;
                    }
                    out.quad_to(a, p);
                    current = p;
                }
            }
            "cubicBezTo" => {
                if let [a, b, p, ..] = pts[..] {
                    if !started {
                        out.move_to(current);
                        started = true;
                    }
                    out.curve_to(a, b, p);
                    current = p;
                }
            }
            "arcTo" => {
                let v = |n: &str| cmd.attr(n).map_or(0.0, |t| guides.get(t));
                let (rx, ry) = (v("wR") * sx, v("hR") * sy);
                let st = v("stAng") / 60000.0 * PI / 180.0;
                let sw = v("swAng") / 60000.0 * PI / 180.0;
                if !started {
                    out.move_to(current);
                    started = true;
                }
                if rx <= 0.0 || ry <= 0.0 {
                    continue;
                }
                let t1 = parametric(st, rx, ry);
                let t2 = parametric(st + sw, rx, ry);
                let center = Point::new(current.x - rx * t1.cos(), current.y - ry * t1.sin());
                let arc = Arc::new(center, (rx, ry), t1, t2 - t1, 0.0);
                for el in arc.append_iter(0.1) {
                    out.push(el);
                }
                current = Point::new(center.x + rx * t2.cos(), center.y + ry * t2.sin());
            }
            "close" => {
                out.close_path();
                started = false;
            }
            _ => {}
        }
    }
    let fill = match path.attr("fill") {
        Some("none") => PathFill::None,
        Some("lighten") => PathFill::Lighten,
        Some("lightenLess") => PathFill::LightenLess,
        Some("darken") => PathFill::Darken,
        Some("darkenLess") => PathFill::DarkenLess,
        _ => PathFill::Normal,
    };
    SubPath {
        path: out,
        fill,
        stroke: !matches!(path.attr("stroke"), Some("0" | "false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn prst(name: &str) -> Element {
        let xml = format!(
            r#"<p:spPr xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:prstGeom prst="{name}"><a:avLst/></a:prstGeom></p:spPr>"#
        );
        Document::parse(xml.as_bytes()).unwrap().root
    }

    #[test]
    fn all_presets_evaluate() {
        assert!(presets().len() > 180, "only {} presets", presets().len());
        for name in presets().keys() {
            let g = shape_geometry(Some(&prst(name)), 200.0, 100.0);
            for p in &g.paths {
                let b = p.path.bounding_box();
                assert!(b.x0.is_finite() && b.y1.is_finite(), "{name}");
                // Shapes stay roughly inside (callouts and arrows may poke out).
                assert!(b.x0 > -400.0 && b.x1 < 600.0, "{name}: {b:?}");
            }
        }
    }

    #[test]
    fn ellipse_is_round() {
        let g = shape_geometry(Some(&prst("ellipse")), 200.0, 100.0);
        let b = g.paths[0].path.bounding_box();
        assert!(
            (b.x0 - 0.0).abs() < 0.5 && (b.x1 - 200.0).abs() < 0.5,
            "{b:?}"
        );
        assert!(
            (b.y0 - 0.0).abs() < 0.5 && (b.y1 - 100.0).abs() < 0.5,
            "{b:?}"
        );
        let area = g.paths[0].path.area().abs();
        assert!((area - PI * 100.0 * 50.0).abs() < 50.0, "area {area}");
    }

    #[test]
    fn round_rect_corners() {
        let g = shape_geometry(Some(&prst("roundRect")), 200.0, 100.0);
        let b = g.paths[0].path.bounding_box();
        assert!((b.width() - 200.0).abs() < 0.5 && (b.height() - 100.0).abs() < 0.5);
        // Default corner radius is 16.667% of the short side.
        let area = g.paths[0].path.area().abs();
        let r = 100.0 * 16667.0 / 100000.0;
        let expected = 200.0 * 100.0 - (4.0 - PI) * r * r;
        assert!((area - expected).abs() < 10.0, "area {area} vs {expected}");
    }
}
