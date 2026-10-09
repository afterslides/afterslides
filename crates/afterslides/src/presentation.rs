use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result, Target};
use crate::opc::{Package, rel_type};
use crate::xml::{Element, ns};

/// Stable handle of a slide: the `id` attribute of its `p:sldId` entry.
///
/// Unlike an index it survives deleting, duplicating and moving other slides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlideId(pub u32);

/// A shape on a slide, identified by its `p:cNvPr` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeRef {
    pub slide: SlideId,
    pub id: u32,
}

/// An open `.pptx` file.
///
/// All editing goes through this type. Slides and shapes are addressed by
/// [`SlideId`] and [`ShapeRef`] handles, which stay valid until the thing
/// they point to is deleted.
#[derive(Debug)]
pub struct Presentation {
    pub(crate) pkg: Package,
    /// Part name of the main presentation part, usually `/ppt/presentation.xml`.
    pub(crate) main: String,
    /// Set when something was removed, so unreferenced parts get dropped on save.
    pub(crate) needs_gc: bool,
}

impl Presentation {
    pub fn open(path: impl AsRef<Path>) -> Result<Presentation> {
        let file = File::open(path)?;
        Presentation::from_reader(BufReader::new(file))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Presentation> {
        Presentation::from_reader(Cursor::new(bytes))
    }

    pub fn from_reader<R: Read + Seek>(reader: R) -> Result<Presentation> {
        let pkg = Package::from_reader(reader)?;
        let main = pkg
            .targets("/")
            .into_iter()
            .find(|(rel, _)| rel.rel_type == rel_type::OFFICE_DOCUMENT)
            .map(|(_, target)| target)
            .ok_or_else(|| Error::Package("no main document relationship".into()))?;
        let doc = pkg.xml(&main)?;
        if !doc.root.is(ns::P, "presentation") {
            return Err(Error::Package(format!(
                "{main} is not a PresentationML document (is this a .docx or .xlsx?)"
            )));
        }
        let mut prs = Presentation {
            pkg,
            main,
            needs_gc: false,
        };
        prs.make_shape_ids_unique()?;
        Ok(prs)
    }

    /// Shapes are addressed by id, so ids must be unique per slide. Files
    /// with duplicates exist (PowerPoint fixes them silently on save); only
    /// such slides are rewritten.
    fn make_shape_ids_unique(&mut self) -> Result<()> {
        for slide in self.slides()? {
            let part = self.slide_part(slide)?;
            if !self.pkg.has_part(&part) {
                continue;
            }
            let tree = crate::shape::sp_tree(&self.pkg.xml(&part)?.root)?;
            if crate::shape::has_duplicate_ids(tree) {
                let tree = crate::shape::sp_tree_mut(&mut self.pkg.xml_mut(&part)?.root)?;
                crate::shape::dedupe_ids(tree);
            }
        }
        Ok(())
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        // Write to a sibling temp file first so a failed save never leaves a
        // truncated deck behind, even when overwriting the template.
        // The name is unique per process and call, so concurrent saves of
        // the same file don't write into each other's temp file.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            ".{}.{}-{}.afterslides-tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("deck"),
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let tmp = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(dir) => dir.join(unique),
            None => unique.into(),
        };
        let result = (|| {
            let file = File::create_new(&tmp)?;
            let mut writer = self.write(BufWriter::new(file))?;
            writer.flush()?;
            Ok::<_, Error>(())
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        Ok(self.write(Cursor::new(Vec::new()))?.into_inner())
    }

    pub fn write<W: Write + Seek>(&mut self, writer: W) -> Result<W> {
        if self.needs_gc {
            self.pkg.collect_garbage();
            self.needs_gc = false;
        }
        self.pkg.write(writer)
    }

    /// Names of all parts in the package, e.g. `/ppt/slides/slide1.xml`.
    pub fn part_names(&self) -> Vec<String> {
        self.pkg.part_names().map(str::to_string).collect()
    }

    // ---- slides -----------------------------------------------------------

    pub(crate) fn sld_id_list(&self) -> Result<Vec<(u32, String)>> {
        let doc = self.pkg.xml(&self.main)?;
        let Some(list) = doc.root.child(ns::P, "sldIdLst") else {
            return Ok(Vec::new());
        };
        list.children_named(ns::P, "sldId")
            .map(|el| {
                let id = el
                    .attr("id")
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| Error::Package("p:sldId without numeric id".into()))?;
                let rid = el
                    .attr_ns(ns::R, "id")
                    .ok_or_else(|| Error::Package("p:sldId without r:id".into()))?;
                Ok((id, rid.to_string()))
            })
            .collect()
    }

    /// All slides, in presentation order.
    pub fn slides(&self) -> Result<Vec<SlideId>> {
        Ok(self
            .sld_id_list()?
            .into_iter()
            .map(|(id, _)| SlideId(id))
            .collect())
    }

    pub fn slide_count(&self) -> Result<usize> {
        Ok(self.sld_id_list()?.len())
    }

    pub fn slide_at(&self, index: usize) -> Result<SlideId> {
        self.slides()?
            .get(index)
            .copied()
            .ok_or(Error::NotFound(Target::SlideIndex(index)))
    }

    pub fn slide_index(&self, slide: SlideId) -> Result<usize> {
        self.slides()?
            .iter()
            .position(|s| *s == slide)
            .ok_or(Error::NotFound(Target::Slide(slide.0)))
    }

    pub fn contains_slide(&self, slide: SlideId) -> bool {
        self.slide_index(slide).is_ok()
    }

    /// Part name of a slide, e.g. `/ppt/slides/slide3.xml`.
    pub fn slide_part(&self, slide: SlideId) -> Result<String> {
        let (_, rid) = self
            .sld_id_list()?
            .into_iter()
            .find(|(id, _)| *id == slide.0)
            .ok_or(Error::NotFound(Target::Slide(slide.0)))?;
        self.pkg.resolve(&self.main, &rid)
    }

    /// Slide size in EMU (`cx`, `cy`).
    pub fn slide_size(&self) -> Result<(i64, i64)> {
        let doc = self.pkg.xml(&self.main)?;
        let size = doc
            .root
            .child(ns::P, "sldSz")
            .ok_or_else(|| Error::Package("presentation has no p:sldSz".into()))?;
        let get = |name| size.attr(name).and_then(|v| v.parse().ok()).unwrap_or(0);
        Ok((get("cx"), get("cy")))
    }

    pub(crate) fn slide_root(&self, slide: SlideId) -> Result<&Element> {
        let part = self.slide_part(slide)?;
        Ok(&self.pkg.xml(&part)?.root)
    }

    pub(crate) fn slide_root_mut(&mut self, slide: SlideId) -> Result<&mut Element> {
        let part = self.slide_part(slide)?;
        Ok(&mut self.pkg.xml_mut(&part)?.root)
    }

    /// Replaces placeholder text on every slide. Returns the number of
    /// replacements made.
    pub fn replace_text(&mut self, replacements: &[(&str, &str)]) -> Result<usize> {
        let mut count = 0;
        for slide in self.slides()? {
            count += self.replace_text_on_slide(slide, replacements)?;
        }
        Ok(count)
    }

    pub fn replace_text_on_slide(
        &mut self,
        slide: SlideId,
        replacements: &[(&str, &str)],
    ) -> Result<usize> {
        let part = self.slide_part(slide)?;
        // Check read-only first so untouched slides stay byte-identical.
        let all_text = self.pkg.xml(&part)?.root.text();
        if !replacements
            .iter()
            .any(|(from, _)| !from.is_empty() && all_text.contains(from))
        {
            return Ok(0);
        }
        Ok(crate::text::replace_in(
            &mut self.pkg.xml_mut(&part)?.root,
            replacements,
        ))
    }
}
