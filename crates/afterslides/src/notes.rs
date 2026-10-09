//! Speaker notes.

use crate::error::{Error, Result};
use crate::opc::{content_type, rel_type, relative_target};
use crate::presentation::{Presentation, SlideId};
use crate::text;
use crate::xml::{Document, Element, ns};

const NOTES_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster";

/// A notes page as PowerPoint creates it: the slide image and the notes body.
const EMPTY_NOTES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#,
    r#"<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/>"#,
    r#"</p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/>"#,
    r#"<p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr>"#,
    r#"<p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>"#,
    r#"<p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/>"#,
    r#"<p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr>"#,
    r#"</p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp>"#,
    r#"</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#,
);

/// The notes body: the `p:sp` holding the body placeholder.
fn body_shape(root: &Element) -> Option<&Element> {
    let tree = root.path(&[(ns::P, "cSld"), (ns::P, "spTree")])?;
    tree.children_named(ns::P, "sp").find(|sp| is_body(sp))
}

fn body_shape_mut(root: &mut Element) -> Option<&mut Element> {
    let tree = root.path_mut(&[(ns::P, "cSld"), (ns::P, "spTree")])?;
    tree.children_named_mut(ns::P, "sp").find(|sp| is_body(sp))
}

fn is_body(sp: &Element) -> bool {
    sp.path(&[(ns::P, "nvSpPr"), (ns::P, "nvPr"), (ns::P, "ph")])
        .is_some_and(|ph| ph.attr("type") == Some("body"))
}

impl Presentation {
    fn notes_part(&self, slide: SlideId) -> Result<Option<String>> {
        let part = self.slide_part(slide)?;
        Ok(self
            .pkg
            .targets(&part)
            .into_iter()
            .find(|(r, t)| r.rel_type == rel_type::NOTES_SLIDE && self.pkg.has_part(t))
            .map(|(_, t)| t))
    }

    /// Speaker notes of a slide; empty if it has none.
    pub fn slide_notes(&self, slide: SlideId) -> Result<String> {
        let Some(part) = self.notes_part(slide)? else {
            return Ok(String::new());
        };
        let root = &self.pkg.xml(&part)?.root;
        Ok(body_shape(root)
            .and_then(|sp| sp.child(ns::P, "txBody"))
            .map(text::get_text)
            .unwrap_or_default())
    }

    /// Replaces the speaker notes of a slide, creating a notes page if needed
    /// (that requires the template to have a notes master).
    pub fn set_slide_notes(&mut self, slide: SlideId, notes: &str) -> Result<()> {
        let part = match self.notes_part(slide)? {
            Some(part) => part,
            None => self.create_notes(slide)?,
        };
        let root = &mut self.pkg.xml_mut(&part)?.root;
        let sp = body_shape_mut(root)
            .ok_or_else(|| Error::Unsupported("the notes page has no notes placeholder".into()))?;
        if sp.child(ns::P, "txBody").is_none() {
            let a = Element {
                name: "a:p".into(),
                ns: Some(ns::A.into()),
                attrs: Vec::new(),
                children: Vec::new(),
            };
            let body = text::new_body(sp, "txBody", &a);
            let at = sp.position(ns::P, "extLst").unwrap_or(sp.children.len());
            sp.children.insert(at, crate::xml::Node::Element(body));
        }
        text::set_text(sp.child_mut(ns::P, "txBody").expect("ensured"), notes);
        Ok(())
    }

    fn create_notes(&mut self, slide: SlideId) -> Result<String> {
        let main = self.main.clone();
        let master = self
            .pkg
            .targets(&main)
            .into_iter()
            .find(|(r, t)| r.rel_type == NOTES_MASTER && self.pkg.has_part(t))
            .map(|(_, t)| t)
            .ok_or_else(|| {
                Error::Unsupported(
                    "the template has no notes master; add speaker notes to any slide in \
                     PowerPoint once"
                        .into(),
                )
            })?;
        let slide_part = self.slide_part(slide)?;
        let notes = self
            .pkg
            .next_part_name("/ppt/notesSlides/notesSlide", ".xml");
        let doc = Document::parse(EMPTY_NOTES.as_bytes())?;
        self.pkg
            .add_xml_part(&notes, content_type::NOTES_SLIDE, doc);
        let rels = self.pkg.rels_mut(&notes);
        rels.add(NOTES_MASTER, &relative_target(&notes, &master));
        rels.add(rel_type::SLIDE, &relative_target(&notes, &slide_part));
        self.pkg
            .rels_mut(&slide_part)
            .add(rel_type::NOTES_SLIDE, &relative_target(&slide_part, &notes));
        Ok(notes)
    }
}
