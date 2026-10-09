//! Deleting, duplicating and reordering slides.
//!
//! A slide is referenced from more places than its `p:sldId` entry: the
//! presentation's relationships, sections (`p14:sectionLst`), custom shows
//! and hyperlinks on other slides. PowerPoint reports a damaged file when any
//! of these points at a slide that no longer exists, so all of them are kept
//! in sync here.

use crate::error::{Error, Result, Target};
use crate::opc::{Relationship, rel_type, relative_target};
use crate::presentation::{Presentation, SlideId};
use crate::xml::{Element, Node, ns};

const COMMENTS_REL: &[&str] = &[
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
    "http://schemas.microsoft.com/office/2018/10/relationships/comments",
];

fn section_lists_mut(root: &mut Element) -> Vec<&mut Element> {
    // p:presentation/p:extLst/p:ext/p14:sectionLst/p14:section/p14:sldIdLst
    let mut out = Vec::new();
    let Some(ext_lst) = root.child_mut(ns::P, "extLst") else {
        return out;
    };
    for ext in ext_lst.children_named_mut(ns::P, "ext") {
        for section_lst in ext.children_named_mut(ns::P14, "sectionLst") {
            for section in section_lst.children_named_mut(ns::P14, "section") {
                if let Some(list) = section.child_mut(ns::P14, "sldIdLst") {
                    out.push(list);
                }
            }
        }
    }
    out
}

fn sld_id_matches(e: &Element, id: u32) -> bool {
    e.attr("id").and_then(|v| v.parse().ok()) == Some(id)
}

/// Puts slide `id` into the section of its neighbour so sections stay
/// contiguous after an insert or move.
fn place_in_sections(root: &mut Element, id: u32, prev: Option<u32>, next: Option<u32>) {
    let mut lists = section_lists_mut(root);
    if lists.is_empty() {
        return;
    }
    let mut template = None;
    for list in lists.iter_mut() {
        if let Some(pos) = list.children.iter().position(
            |n| matches!(n, Node::Element(e) if e.is(ns::P14, "sldId") && sld_id_matches(e, id)),
        ) {
            template = Some(list.children.remove(pos));
        }
    }
    let entry = template.unwrap_or_else(|| {
        let like = lists
            .iter()
            .flat_map(|l| l.elements())
            .next()
            .cloned()
            .unwrap_or_else(|| Element::new_like(lists[0], "sldId"));
        let mut e = Element::new_like(&like, "sldId");
        e.set_attr("id", id.to_string());
        Node::Element(e)
    });

    let find = |lists: &[&mut Element], target: u32| {
        lists.iter().enumerate().find_map(|(li, list)| {
            list.children
                .iter()
                .position(|n| matches!(n, Node::Element(e) if e.is(ns::P14, "sldId") && sld_id_matches(e, target)))
                .map(|pos| (li, pos))
        })
    };
    let (li, pos) = prev
        .and_then(|p| find(&lists, p).map(|(li, pos)| (li, pos + 1)))
        .or_else(|| next.and_then(|n| find(&lists, n)))
        .unwrap_or((0, 0));
    lists[li].children.insert(pos, entry);
}

impl Presentation {
    fn sld_id_lst_mut(&mut self) -> Result<&mut Element> {
        let main = self.main.clone();
        self.pkg
            .xml_mut(&main)?
            .root
            .child_mut(ns::P, "sldIdLst")
            .ok_or_else(|| Error::Package("presentation has no slides".into()))
    }

    fn update_app_slide_count(&mut self) -> Result<()> {
        let Some((_, app)) = self
            .pkg
            .targets("/")
            .into_iter()
            .find(|(r, _)| r.rel_type == rel_type::EXTENDED_PROPERTIES)
        else {
            return Ok(());
        };
        if !self.pkg.has_part(&app) {
            return Ok(());
        }
        let count = self.slide_count()?;
        let doc = self.pkg.xml_mut(&app)?;
        if let Some(slides) = doc.root.elements_mut().find(|e| e.local() == "Slides") {
            slides.set_text(count.to_string());
        }
        Ok(())
    }

    /// Removes a slide together with its notes, charts and any media that no
    /// other slide uses.
    pub fn delete_slide(&mut self, slide: SlideId) -> Result<()> {
        let (_, rid) = self
            .sld_id_list()?
            .into_iter()
            .find(|(id, _)| *id == slide.0)
            .ok_or(Error::NotFound(Target::Slide(slide.0)))?;
        let part = self.pkg.resolve(&self.main, &rid)?;
        let main = self.main.clone();

        let root = &mut self.pkg.xml_mut(&main)?.root;
        let list = root
            .child_mut(ns::P, "sldIdLst")
            .expect("slide was found in the list");
        list.children
            .retain(|n| !matches!(n, Node::Element(e) if e.is(ns::P, "sldId") && sld_id_matches(e, slide.0)));
        for list in section_lists_mut(root) {
            list.children.retain(
                |n| !matches!(n, Node::Element(e) if e.is(ns::P14, "sldId") && sld_id_matches(e, slide.0)),
            );
        }
        if let Some(shows) = root.child_mut(ns::P, "custShowLst") {
            shows.remove_descendants(&|e| {
                e.is(ns::P, "sld") && e.attr_ns(ns::R, "id") == Some(rid.as_str())
            });
        }
        self.pkg.rels_mut(&main).remove(&rid);

        // Hyperlinks (and anything else) on other parts that jump to this slide.
        let sources: Vec<String> = self.pkg.part_names().map(str::to_string).collect();
        for source in sources {
            if source == part {
                continue;
            }
            let dangling: Vec<String> = self
                .pkg
                .targets(&source)
                .into_iter()
                .filter(|(r, target)| *target == part && r.rel_type == rel_type::SLIDE)
                .map(|(r, _)| r.id)
                .collect();
            if dangling.is_empty() {
                continue;
            }
            // Notes slides point back at their slide; they go away with it.
            if self.pkg.content_type(&source).as_deref()
                == Some(crate::opc::content_type::NOTES_SLIDE)
            {
                continue;
            }
            let doc = self.pkg.xml_mut(&source)?;
            doc.root.remove_descendants(&|e| {
                e.attrs
                    .iter()
                    .any(|a| a.ns.as_deref() == Some(ns::R) && dangling.contains(&a.value))
            });
            let rels = self.pkg.rels_mut(&source);
            for id in &dangling {
                rels.remove(id);
            }
        }

        self.needs_gc = true;
        self.update_app_slide_count()
    }

    /// Copies a slide and inserts the copy at `position` (default: right after
    /// the original). Charts and notes are deep-copied so the copy can be
    /// filled independently; layouts and images are shared.
    pub fn duplicate_slide(&mut self, slide: SlideId, position: Option<usize>) -> Result<SlideId> {
        let src = self.slide_part(slide)?;
        let src_index = self.slide_index(slide)?;
        let count = self.slide_count()?;
        let position = position.unwrap_or(src_index + 1);
        if position > count {
            return Err(Error::InvalidArgument(format!(
                "position {position} is beyond the end of a {count}-slide deck"
            )));
        }

        let new_part = self.pkg.next_part_name("/ppt/slides/slide", ".xml");
        let ct = self
            .pkg
            .content_type(&src)
            .unwrap_or_else(|| crate::opc::content_type::SLIDE.into());
        let doc = self.pkg.xml(&src)?.clone();
        self.pkg.add_xml_part(&new_part, &ct, doc);

        let mut dropped = Vec::new();
        for (rel, target) in self.pkg.targets(&src) {
            // Comments belong to the original slide.
            if COMMENTS_REL.contains(&rel.rel_type.as_str()) {
                dropped.push(rel.id);
                continue;
            }
            let new_target = match rel.rel_type.as_str() {
                rel_type::CHART => Some(self.deep_copy_part(&target)?),
                rel_type::NOTES_SLIDE => {
                    let copy = self.copy_part(&target, "/ppt/notesSlides/notesSlide", ".xml")?;
                    // The copied notes still point at the original slide.
                    let back: Vec<String> = self
                        .pkg
                        .targets(&copy)
                        .into_iter()
                        .filter(|(r, t)| r.rel_type == rel_type::SLIDE && *t == src)
                        .map(|(r, _)| r.id)
                        .collect();
                    let rel_target = relative_target(&copy, &new_part);
                    for id in back {
                        self.pkg.rels_mut(&copy).set_target(&id, &rel_target);
                    }
                    Some(copy)
                }
                _ => None,
            };
            let target = match new_target {
                Some(t) => relative_target(&new_part, &t),
                None => rel.target.clone(),
            };
            self.pkg
                .rels_mut(&new_part)
                .push(Relationship { target, ..rel });
        }
        // External relationships (web links) are not in `targets`.
        let external: Vec<Relationship> = self
            .pkg
            .rels(&src)
            .into_iter()
            .flat_map(|r| r.iter())
            .filter(|r| r.external)
            .cloned()
            .collect();
        for rel in external {
            self.pkg.rels_mut(&new_part).push(rel);
        }

        let root = &mut self.pkg.xml_mut(&new_part)?.root;
        strip_references(root, &dropped);
        renew_creation_id(root);

        let main = self.main.clone();
        let rid = self
            .pkg
            .rels_mut(&main)
            .add(rel_type::SLIDE, &relative_target(&main, &new_part));
        let ids = self.sld_id_list()?;
        let new_id = ids.iter().map(|(id, _)| *id).max().unwrap_or(255).max(255) + 1;
        let prev = position.checked_sub(1).map(|i| ids[i].0);
        let next = ids.get(position).map(|(id, _)| *id);

        let list = self.sld_id_lst_mut()?;
        let template = list
            .children_named(ns::P, "sldId")
            .find(|e| sld_id_matches(e, slide.0))
            .cloned()
            .expect("source slide is in the list");
        let mut entry = template;
        entry.set_attr("id", new_id.to_string());
        entry.set_attr_ns(ns::R, "id", rid);
        let insert_at = match next {
            Some(n) => list
                .children
                .iter()
                .position(|c| matches!(c, Node::Element(e) if e.is(ns::P, "sldId") && sld_id_matches(e, n)))
                .expect("neighbour is in the list"),
            None => list
                .children
                .iter()
                .rposition(|c| matches!(c, Node::Element(e) if e.is(ns::P, "sldId")))
                .map_or(list.children.len(), |p| p + 1),
        };
        list.children.insert(insert_at, Node::Element(entry));
        let root = &mut self.pkg.xml_mut(&main)?.root;
        place_in_sections(root, new_id, prev, next);

        self.update_app_slide_count()?;
        Ok(SlideId(new_id))
    }

    /// Whether the slide is hidden in slide shows (`show="0"`).
    pub fn slide_hidden(&self, slide: SlideId) -> Result<bool> {
        Ok(matches!(
            self.slide_root(slide)?.attr("show"),
            Some("0" | "false")
        ))
    }

    pub fn set_slide_hidden(&mut self, slide: SlideId, hidden: bool) -> Result<()> {
        if self.slide_hidden(slide)? == hidden {
            return Ok(());
        }
        let root = self.slide_root_mut(slide)?;
        if hidden {
            root.set_attr("show", "0");
        } else {
            root.remove_attr("show");
        }
        Ok(())
    }

    /// Moves a slide to a new position (0-based, counted after removal).
    pub fn move_slide(&mut self, slide: SlideId, position: usize) -> Result<()> {
        let count = self.slide_count()?;
        if position >= count {
            return Err(Error::InvalidArgument(format!(
                "position {position} is beyond the end of a {count}-slide deck"
            )));
        }
        let list = self.sld_id_lst_mut()?;
        let from = list
            .children
            .iter()
            .position(|c| matches!(c, Node::Element(e) if e.is(ns::P, "sldId") && sld_id_matches(e, slide.0)))
            .ok_or(Error::NotFound(Target::Slide(slide.0)))?;
        let entry = list.children.remove(from);
        let slots: Vec<usize> = list
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Node::Element(e) if e.is(ns::P, "sldId")))
            .map(|(i, _)| i)
            .collect();
        let at = match slots.get(position) {
            Some(&i) => i,
            None => slots.last().map_or(list.children.len(), |&i| i + 1),
        };
        list.children.insert(at, entry);

        let ids = self.slides()?;
        let prev = position.checked_sub(1).map(|i| ids[i].0);
        let next = ids.get(position + 1).map(|s| s.0);
        let main = self.main.clone();
        let root = &mut self.pkg.xml_mut(&main)?.root;
        place_in_sections(root, slide.0, prev, next);
        Ok(())
    }

    /// Copies a part and its relationships (targets are shared) under a new
    /// name `{prefix}{n}{suffix}`. Returns the new part name.
    fn copy_part(&mut self, src: &str, prefix: &str, suffix: &str) -> Result<String> {
        let name = self.pkg.next_part_name(prefix, suffix);
        let ct = self
            .pkg
            .content_type(src)
            .ok_or_else(|| Error::Package(format!("{src} has no content type")))?;
        let data = self.pkg.raw(src)?;
        self.pkg.add_raw_part(&name, &ct, data);
        let rels: Vec<Relationship> = self
            .pkg
            .rels(src)
            .into_iter()
            .flat_map(|r| r.iter())
            .cloned()
            .collect();
        for rel in rels {
            let target = if rel.external {
                rel.target.clone()
            } else {
                relative_target(&name, &crate::opc::resolve_target(src, &rel.target))
            };
            self.pkg
                .rels_mut(&name)
                .push(Relationship { target, ..rel });
        }
        Ok(name)
    }

    /// Copies a part and, recursively, every part it owns (a chart with its
    /// workbook, style and colour parts).
    fn deep_copy_part(&mut self, src: &str) -> Result<String> {
        let (prefix, suffix) = split_numbered(src);
        let copy = self.copy_part(src, &prefix, &suffix)?;
        for (rel, target) in self.pkg.targets(src) {
            if !self.pkg.has_part(&target) {
                continue;
            }
            let (prefix, suffix) = split_numbered(&target);
            let child = self.copy_part(&target, &prefix, &suffix)?;
            let rel_target = relative_target(&copy, &child);
            self.pkg.rels_mut(&copy).set_target(&rel.id, &rel_target);
        }
        Ok(copy)
    }
}

/// Removes elements that refer to the given relationship ids (such as
/// `p188:commentRel`), and extension containers left empty by that.
fn strip_references(root: &mut Element, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    root.remove_descendants(&|e| {
        e.attrs
            .iter()
            .any(|a| a.ns.as_deref() == Some(ns::R) && ids.contains(&a.value))
    });
    root.remove_descendants(&|e| e.is(ns::P, "ext") && e.elements().next().is_none());
    root.remove_descendants(&|e| e.is(ns::P, "extLst") && e.elements().next().is_none());
}

/// Gives a copied slide its own `p14:creationId`; PowerPoint uses it to tell
/// slides apart when merging and comparing decks.
fn renew_creation_id(root: &mut Element) {
    use std::hash::BuildHasher;
    let fresh = std::collections::hash_map::RandomState::new().hash_one(root.text().len()) as u32;
    root.walk_mut(&mut |e| {
        if e.is(ns::P14, "creationId") {
            e.set_attr("val", fresh.max(1).to_string());
        }
    });
}

/// `/ppt/charts/chart12.xml` -> (`/ppt/charts/chart`, `.xml`).
fn split_numbered(name: &str) -> (String, String) {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > name.rfind('/').unwrap_or(0) => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let base = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let base = base.strip_suffix('_').unwrap_or(base);
    (base.to_string(), ext.to_string())
}

#[cfg(test)]
mod tests {
    use super::split_numbered;

    #[test]
    fn numbered_names() {
        assert_eq!(
            split_numbered("/ppt/charts/chart12.xml"),
            ("/ppt/charts/chart".into(), ".xml".into())
        );
        assert_eq!(
            split_numbered("/ppt/embeddings/Microsoft_Excel_Worksheet3.xlsx"),
            (
                "/ppt/embeddings/Microsoft_Excel_Worksheet".into(),
                ".xlsx".into()
            )
        );
        assert_eq!(
            split_numbered("/ppt/charts/style1.xml"),
            ("/ppt/charts/style".into(), ".xml".into())
        );
    }
}
