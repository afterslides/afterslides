//! The shape tree of a slide (`p:spTree`).

use std::collections::HashSet;

use crate::error::{Error, Result, Target};
use crate::presentation::{Presentation, ShapeRef, SlideId};
use crate::text;
use crate::xml::{Element, Node, ns};

const URI_TABLE: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";
const URI_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const URI_CHARTEX: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";
const URI_DIAGRAM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const URI_OLE: &str = "http://schemas.openxmlformats.org/presentationml/2006/ole";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapeKind {
    /// `p:sp`: rectangles, text boxes, placeholders and all other auto shapes.
    Shape,
    Picture,
    Table,
    Chart,
    /// Office 2016+ charts (waterfall, treemap, ...); data editing is not supported.
    ChartEx,
    SmartArt,
    OleObject,
    Group,
    Connector,
    /// Any other graphic frame, or a content part (ink).
    Other,
}

impl ShapeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShapeKind::Shape => "shape",
            ShapeKind::Picture => "picture",
            ShapeKind::Table => "table",
            ShapeKind::Chart => "chart",
            ShapeKind::ChartEx => "chartex",
            ShapeKind::SmartArt => "smartart",
            ShapeKind::OleObject => "ole_object",
            ShapeKind::Group => "group",
            ShapeKind::Connector => "connector",
            ShapeKind::Other => "other",
        }
    }
}

/// Placeholder info from `p:nvPr/p:ph`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Placeholder {
    /// `title`, `body`, `ctrTitle`, `pic`, ...; `body` when not given, as per the spec.
    pub kind: String,
    pub idx: u32,
}

/// A snapshot of a shape's identifying properties.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ShapeInfo {
    pub shape: ShapeRef,
    pub name: String,
    /// Alt text (`descr`), a common place to put template markers.
    pub alt_text: String,
    pub title: String,
    pub kind: ShapeKind,
    pub placeholder: Option<Placeholder>,
    pub has_text: bool,
    pub hidden: bool,
    /// Id of the enclosing group shape, if any.
    pub parent: Option<u32>,
    /// Position and size in EMU: (x, y, cx, cy), if the shape has its own transform.
    pub frame: Option<(i64, i64, i64, i64)>,
}

fn is_shape_element(e: &Element) -> bool {
    e.ns() == Some(ns::P)
        && matches!(
            e.local(),
            "sp" | "pic" | "graphicFrame" | "grpSp" | "cxnSp" | "contentPart"
        )
}

/// `p:cNvPr` of a shape element.
pub(crate) fn c_nv_pr(shape: &Element) -> Option<&Element> {
    shape
        .elements()
        .find(|e| e.ns() == Some(ns::P) && e.local().starts_with("nv"))
        .and_then(|nv| nv.child(ns::P, "cNvPr"))
}

pub(crate) fn shape_id(shape: &Element) -> Option<u32> {
    c_nv_pr(shape)?.attr("id")?.parse().ok()
}

fn graphic_data(shape: &Element) -> Option<&Element> {
    shape.path(&[(ns::A, "graphic"), (ns::A, "graphicData")])
}

pub(crate) fn kind_of(shape: &Element) -> ShapeKind {
    match shape.local() {
        "sp" => ShapeKind::Shape,
        "pic" => ShapeKind::Picture,
        "grpSp" => ShapeKind::Group,
        "cxnSp" => ShapeKind::Connector,
        "graphicFrame" => match graphic_data(shape).and_then(|g| g.attr("uri")) {
            Some(URI_TABLE) => ShapeKind::Table,
            Some(URI_CHART) => ShapeKind::Chart,
            Some(URI_CHARTEX) => ShapeKind::ChartEx,
            Some(URI_DIAGRAM) => ShapeKind::SmartArt,
            Some(URI_OLE) => ShapeKind::OleObject,
            _ => ShapeKind::Other,
        },
        _ => ShapeKind::Other,
    }
}

fn frame_of(shape: &Element) -> Option<(i64, i64, i64, i64)> {
    let xfrm = match shape.local() {
        "graphicFrame" => shape.child(ns::P, "xfrm"),
        "grpSp" => shape.path(&[(ns::P, "grpSpPr"), (ns::A, "xfrm")]),
        _ => shape.path(&[(ns::P, "spPr"), (ns::A, "xfrm")]),
    }?;
    let off = xfrm.child(ns::A, "off")?;
    let ext = xfrm.child(ns::A, "ext")?;
    let num = |e: &Element, name| e.attr(name).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some((num(off, "x"), num(off, "y"), num(ext, "cx"), num(ext, "cy")))
}

fn info(slide: SlideId, shape: &Element, parent: Option<u32>) -> Option<ShapeInfo> {
    let c = c_nv_pr(shape)?;
    let id = c.attr("id")?.parse().ok()?;
    let placeholder = shape
        .elements()
        .find(|e| e.ns() == Some(ns::P) && e.local().starts_with("nv"))
        .and_then(|nv| nv.path(&[(ns::P, "nvPr"), (ns::P, "ph")]))
        .map(|ph| Placeholder {
            kind: ph.attr("type").unwrap_or("body").to_string(),
            idx: ph.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0),
        });
    Some(ShapeInfo {
        shape: ShapeRef { slide, id },
        name: c.attr("name").unwrap_or_default().to_string(),
        alt_text: c.attr("descr").unwrap_or_default().to_string(),
        title: c.attr("title").unwrap_or_default().to_string(),
        kind: kind_of(shape),
        placeholder,
        has_text: shape.child(ns::P, "txBody").is_some(),
        hidden: matches!(c.attr("hidden"), Some("1" | "true")),
        parent,
        frame: frame_of(shape),
    })
}

/// Visits shapes in document order, descending into groups. For
/// `mc:AlternateContent` only the first choice is reported, since that is
/// what PowerPoint shows.
fn visit<'a>(
    container: &'a Element,
    parent: Option<u32>,
    f: &mut impl FnMut(&'a Element, Option<u32>),
) {
    for child in container.elements() {
        if is_shape_element(child) {
            f(child, parent);
            if child.local() == "grpSp" {
                visit(child, shape_id(child), f);
            }
        } else if child.is(ns::MC, "AlternateContent")
            && let Some(choice) = child
                .elements()
                .find(|e| e.is(ns::MC, "Choice") || e.is(ns::MC, "Fallback"))
        {
            visit(choice, parent, f);
        }
    }
}

/// Paths (child indices from the shape tree) of every element that
/// represents shape `id`, including `mc:Fallback` copies.
fn paths_of(sp_tree: &Element, id: u32) -> Vec<Vec<usize>> {
    fn walk(el: &Element, id: u32, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, child) in el.children.iter().enumerate() {
            let Node::Element(child) = child else {
                continue;
            };
            path.push(i);
            if is_shape_element(child) && shape_id(child) == Some(id) {
                out.push(path.clone());
            } else if child.local() == "grpSp"
                || child.is(ns::MC, "AlternateContent")
                || child.is(ns::MC, "Choice")
                || child.is(ns::MC, "Fallback")
            {
                walk(child, id, path, out);
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(sp_tree, id, &mut Vec::new(), &mut out);
    out
}

fn at_path<'a>(root: &'a Element, path: &[usize]) -> &'a Element {
    path.iter().fold(root, |el, &i| match &el.children[i] {
        Node::Element(e) => e,
        _ => unreachable!("paths only point at elements"),
    })
}

fn at_path_mut<'a>(root: &'a mut Element, path: &[usize]) -> &'a mut Element {
    path.iter().fold(root, |el, &i| match &mut el.children[i] {
        Node::Element(e) => e,
        _ => unreachable!("paths only point at elements"),
    })
}

fn c_nv_pr_mut(shape: &mut Element) -> Option<&mut Element> {
    shape
        .elements_mut()
        .find(|e| e.ns() == Some(ns::P) && e.local().starts_with("nv"))
        .and_then(|nv| nv.child_mut(ns::P, "cNvPr"))
}

/// Ids of all shapes PowerPoint shows (fallback copies excluded), plus the
/// id of the shape tree itself.
fn visible_ids(sp_tree: &Element) -> Vec<u32> {
    let mut ids: Vec<u32> = shape_id(sp_tree).into_iter().collect();
    visit(sp_tree, None, &mut |el, _| ids.extend(shape_id(el)));
    ids
}

pub(crate) fn has_duplicate_ids(sp_tree: &Element) -> bool {
    let ids = visible_ids(sp_tree);
    let unique: HashSet<u32> = ids.iter().copied().collect();
    unique.len() != ids.len()
}

/// Gives every shape after the first one with a given id a fresh id, like
/// PowerPoint does when it saves such a file. The copy of a shape in
/// `mc:Fallback` follows its `mc:Choice` twin.
pub(crate) fn dedupe_ids(sp_tree: &mut Element) {
    let mut next = visible_ids(sp_tree).into_iter().max().unwrap_or(1);
    let mut seen: HashSet<u32> = shape_id(sp_tree).into_iter().collect();

    fn walk(container: &mut Element, seen: &mut HashSet<u32>, next: &mut u32) {
        for child in container.elements_mut() {
            if is_shape_element(child) {
                if let Some(id) = shape_id(child)
                    && !seen.insert(id)
                {
                    *next += 1;
                    seen.insert(*next);
                    if let Some(c) = c_nv_pr_mut(child) {
                        c.set_attr("id", next.to_string());
                    }
                }
                if child.local() == "grpSp" {
                    walk(child, seen, next);
                }
            } else if child.is(ns::MC, "AlternateContent") {
                // Renumber the first branch, then apply the same mapping to
                // the others so twins keep matching ids.
                let mut branches = child
                    .elements_mut()
                    .filter(|e| e.is(ns::MC, "Choice") || e.is(ns::MC, "Fallback"));
                let Some(first) = branches.next() else {
                    continue;
                };
                let before = branch_ids(first);
                walk(first, seen, next);
                let after = branch_ids(first);
                let map: Vec<(u32, u32)> = before.into_iter().zip(after).collect();
                for other in branches {
                    other.walk_mut(&mut |e| {
                        if is_shape_element(e)
                            && let Some(id) = shape_id(e)
                            && let Some((_, new)) = map.iter().find(|(old, _)| *old == id)
                            && let Some(c) = c_nv_pr_mut(e)
                        {
                            c.set_attr("id", new.to_string());
                        }
                    });
                }
            }
        }
    }

    fn branch_ids(branch: &Element) -> Vec<u32> {
        let mut ids = Vec::new();
        branch.walk(&mut |e| {
            if is_shape_element(e) {
                ids.extend(shape_id(e));
            }
        });
        ids
    }

    walk(sp_tree, &mut seen, &mut next);
}

/// Removes what still points at deleted shapes: animations and build steps
/// in `p:timing`, and connector ends glued to them. PowerPoint repairs a
/// file whose animations target a shape that no longer exists.
fn drop_shape_references(slide_root: &mut Element, ids: &HashSet<String>) {
    let targets_deleted = |e: &Element| {
        let mut hit = false;
        e.walk(&mut |x| {
            hit |= x.is(ns::P, "spTgt") && x.attr("spid").is_some_and(|id| ids.contains(id));
        });
        hit
    };

    // Connectors keep their geometry but are no longer attached.
    slide_root.remove_descendants(&|e| {
        (e.is(ns::A, "stCxn") || e.is(ns::A, "endCxn"))
            && e.attr("id").is_some_and(|id| ids.contains(id))
    });

    let Some(timing) = slide_root.child_mut(ns::P, "timing") else {
        return;
    };
    if let Some(builds) = timing.child_mut(ns::P, "bldLst") {
        builds.remove_descendants(&|e| e.attr("spid").is_some_and(|id| ids.contains(id)));
    }
    if timing
        .child(ns::P, "bldLst")
        .is_some_and(|b| b.elements().next().is_none())
    {
        timing.remove_children(ns::P, "bldLst");
    }

    // An effect is a p:par whose time node has a preset class (entrance,
    // emphasis, exit, path).
    let is_effect = |e: &Element| {
        e.is(ns::P, "par")
            && e.child(ns::P, "cTn")
                .is_some_and(|c| c.attr("presetClass").is_some())
    };
    let removed = timing.remove_descendants(&|e| is_effect(e) && targets_deleted(e));

    // Click groups that lost all their effects go too, innermost first.
    let empty_group = |e: &Element| {
        e.is(ns::P, "par")
            && e.path(&[(ns::P, "cTn"), (ns::P, "childTnLst")])
                .is_some_and(|l| l.elements().next().is_none())
    };
    while timing.remove_descendants(&empty_group) > 0 {}

    // If anything still refers to a deleted shape (triggers, odd
    // structures), or no animation is left, drop the slide's animations
    // rather than leave a broken timing tree.
    let mut any_target = false;
    timing.walk(&mut |e| any_target |= e.is(ns::P, "spTgt"));
    let empty_list = {
        let mut empty = false;
        timing.walk(&mut |e| {
            empty |= e.is(ns::P, "childTnLst") && e.elements().next().is_none();
        });
        empty
    };
    if targets_deleted(timing) || (removed > 0 && (!any_target || empty_list)) {
        slide_root.remove_children(ns::P, "timing");
    }
}

pub(crate) fn sp_tree(slide_root: &Element) -> Result<&Element> {
    slide_root
        .path(&[(ns::P, "cSld"), (ns::P, "spTree")])
        .ok_or_else(|| Error::Package("slide has no p:spTree".into()))
}

pub(crate) fn sp_tree_mut(slide_root: &mut Element) -> Result<&mut Element> {
    slide_root
        .path_mut(&[(ns::P, "cSld"), (ns::P, "spTree")])
        .ok_or_else(|| Error::Package("slide has no p:spTree".into()))
}

/// Collects every `r:*` attribute value in a subtree.
pub(crate) fn rel_ids(el: &Element, out: &mut HashSet<String>) {
    el.walk(&mut |e| {
        for a in &e.attrs {
            if a.ns.as_deref() == Some(ns::R) {
                out.insert(a.value.clone());
            }
        }
    });
}

impl Presentation {
    /// All shapes on a slide, groups included (followed by their members).
    pub fn shapes(&self, slide: SlideId) -> Result<Vec<ShapeInfo>> {
        let tree = sp_tree(self.slide_root(slide)?)?;
        let mut out = Vec::new();
        visit(tree, None, &mut |el, parent| {
            if let Some(i) = info(slide, el, parent) {
                out.push(i);
            }
        });
        Ok(out)
    }

    pub fn shape_info(&self, shape: ShapeRef) -> Result<ShapeInfo> {
        self.shapes(shape.slide)?
            .into_iter()
            .find(|s| s.shape == shape)
            .ok_or(Error::NotFound(Target::Shape {
                slide: shape.slide.0,
                shape: shape.id,
            }))
    }

    /// First shape on the slide with the given name.
    pub fn find_shape(&self, slide: SlideId, name: &str) -> Result<Option<ShapeRef>> {
        Ok(self
            .shapes(slide)?
            .into_iter()
            .find(|s| s.name == name)
            .map(|s| s.shape))
    }

    /// The element of a shape (the first choice if it is wrapped in
    /// `mc:AlternateContent`).
    pub(crate) fn shape_element(&self, shape: ShapeRef) -> Result<&Element> {
        let tree = sp_tree(self.slide_root(shape.slide)?)?;
        let path = paths_of(tree, shape.id)
            .into_iter()
            .next()
            .ok_or(Error::NotFound(Target::Shape {
                slide: shape.slide.0,
                shape: shape.id,
            }))?;
        Ok(at_path(tree, &path))
    }

    /// Applies `f` to every element representing the shape (normally one;
    /// two when PowerPoint stored a fallback copy).
    pub(crate) fn with_shape_mut<T>(
        &mut self,
        shape: ShapeRef,
        mut f: impl FnMut(&mut Element) -> Result<T>,
    ) -> Result<T> {
        let tree = sp_tree_mut(self.slide_root_mut(shape.slide)?)?;
        let paths = paths_of(tree, shape.id);
        if paths.is_empty() {
            return Err(Error::NotFound(Target::Shape {
                slide: shape.slide.0,
                shape: shape.id,
            }));
        }
        let mut result = None;
        for path in paths {
            let r = f(at_path_mut(tree, &path))?;
            result.get_or_insert(r);
        }
        Ok(result.expect("at least one path"))
    }

    /// Text of a shape, or `None` if it cannot hold text.
    pub fn shape_text(&self, shape: ShapeRef) -> Result<Option<String>> {
        let el = self.shape_element(shape)?;
        Ok(el.child(ns::P, "txBody").map(text::get_text))
    }

    /// Replaces the text of a shape, keeping the formatting of its first run.
    pub fn set_shape_text(&mut self, shape: ShapeRef, value: &str) -> Result<()> {
        self.with_shape_mut(shape, |el| {
            if !el.is(ns::P, "sp") {
                return Err(Error::Unsupported(format!(
                    "shape {} is a {}, which has no text frame",
                    shape.id,
                    kind_of(el).as_str()
                )));
            }
            if el.child(ns::P, "txBody").is_none() {
                let a_like = el
                    .elements()
                    .flat_map(|e| e.elements())
                    .find(|e| e.ns() == Some(ns::A))
                    .cloned()
                    .unwrap_or_else(|| Element {
                        name: "a:p".into(),
                        ns: Some(ns::A.into()),
                        attrs: Vec::new(),
                        children: Vec::new(),
                    });
                let body = text::new_body(el, "txBody", &a_like);
                // Schema order: nvSpPr, spPr, style, txBody, extLst.
                let at = el.position(ns::P, "extLst").unwrap_or(el.children.len());
                el.children.insert(at, Node::Element(body));
            }
            text::set_text(el.child_mut(ns::P, "txBody").expect("just ensured"), value);
            Ok(())
        })
    }

    /// Replaces placeholder text in one shape (text frames and table cells).
    pub fn replace_text_in_shape(
        &mut self,
        shape: ShapeRef,
        replacements: &[(&str, &str)],
    ) -> Result<usize> {
        self.with_shape_mut(shape, |el| Ok(text::replace_in(el, replacements)))
    }

    /// Removes a shape from its slide. Charts, images and other parts it
    /// referenced are dropped on save if nothing else uses them.
    pub fn delete_shape(&mut self, shape: ShapeRef) -> Result<()> {
        let part = self.slide_part(shape.slide)?;
        let root = &mut self.pkg.xml_mut(&part)?.root;
        let tree = sp_tree_mut(root)?;
        let paths = paths_of(tree, shape.id);
        if paths.is_empty() {
            return Err(Error::NotFound(Target::Shape {
                slide: shape.slide.0,
                shape: shape.id,
            }));
        }
        // A shape inside mc:Choice/mc:Fallback takes the whole
        // mc:AlternateContent with it.
        let mut targets: Vec<Vec<usize>> = paths
            .into_iter()
            .map(|mut path| {
                if path.len() >= 2 {
                    let parent = at_path(tree, &path[..path.len() - 1]);
                    if parent.is(ns::MC, "Choice") || parent.is(ns::MC, "Fallback") {
                        path.truncate(path.len() - 2);
                    }
                }
                path
            })
            .collect();
        targets.sort();
        targets.dedup();

        let mut removed_ids = HashSet::new();
        let mut removed_shapes = HashSet::new();
        // Delete from the back so earlier paths stay valid.
        for path in targets.iter().rev() {
            let (last, parent_path) = path.split_last().expect("non-empty path");
            let parent = at_path_mut(tree, parent_path);
            if let Node::Element(e) = parent.children.remove(*last) {
                rel_ids(&e, &mut removed_ids);
                e.walk(&mut |el| {
                    if is_shape_element(el) {
                        removed_shapes.extend(shape_id(el).map(|id| id.to_string()));
                    }
                });
            }
        }
        drop_shape_references(root, &removed_shapes);

        let mut still_used = HashSet::new();
        rel_ids(root, &mut still_used);
        let orphaned: Vec<String> = removed_ids.difference(&still_used).cloned().collect();
        if !orphaned.is_empty() {
            let rels = self.pkg.rels_mut(&part);
            for id in orphaned {
                rels.remove(&id);
            }
            self.needs_gc = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Document;

    #[test]
    fn dedupe_keeps_alternate_content_twins_in_sync() {
        let xml = r#"<p:spTree xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
            <p:nvGrpSpPr><p:cNvPr id="1" name=""/></p:nvGrpSpPr>
            <p:sp><p:nvSpPr><p:cNvPr id="2" name="a"/></p:nvSpPr></p:sp>
            <mc:AlternateContent>
              <mc:Choice Requires="p14"><p:sp><p:nvSpPr><p:cNvPr id="2" name="b"/></p:nvSpPr></p:sp></mc:Choice>
              <mc:Fallback><p:sp><p:nvSpPr><p:cNvPr id="2" name="b"/></p:nvSpPr></p:sp></mc:Fallback>
            </mc:AlternateContent>
            <p:grpSp><p:nvGrpSpPr><p:cNvPr id="3" name="g"/></p:nvGrpSpPr>
              <p:sp><p:nvSpPr><p:cNvPr id="2" name="c"/></p:nvSpPr></p:sp>
            </p:grpSp>
        </p:spTree>"#;
        let mut doc = Document::parse(xml.as_bytes()).unwrap();
        assert!(has_duplicate_ids(&doc.root));
        dedupe_ids(&mut doc.root);
        assert!(!has_duplicate_ids(&doc.root));
        let mut names = Vec::new();
        doc.root.walk(&mut |e| {
            if is_shape_element(e) {
                let c = c_nv_pr(e).unwrap();
                names.push(format!(
                    "{}={}",
                    c.attr("name").unwrap(),
                    c.attr("id").unwrap()
                ));
            }
        });
        assert_eq!(names, ["a=2", "b=4", "b=4", "g=3", "c=5"]);
    }
}
