//! Open Packaging Conventions: the zip container, content types and
//! relationships that every OOXML file is built on.
//!
//! Parts are kept as raw bytes until something asks for their XML. Parts that
//! were never touched are written back byte for byte.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::{Cursor, Read, Seek, Write};
use std::sync::OnceLock;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result, Target};
use crate::xml::{Attr, Document, Element, Node, ns};

const CONTENT_TYPES: &str = "[Content_Types].xml";

/// Refuse parts that inflate beyond this; protects against zip bombs.
pub const MAX_PART_SIZE: u64 = 512 * 1024 * 1024;
/// Refuse packages whose parts add up to more than this.
pub const MAX_PACKAGE_SIZE: u64 = 2 * 1024 * 1024 * 1024;

pub mod rel_type {
    pub const OFFICE_DOCUMENT: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    pub const SLIDE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
    pub const NOTES_SLIDE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
    pub const CHART: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
    pub const PACKAGE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";
    pub const EXTENDED_PROPERTIES: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
}

pub mod content_type {
    pub const SLIDE: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
    pub const NOTES_SLIDE: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
    pub const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    /// Target as written in the rels part (usually relative).
    pub target: String,
    pub external: bool,
}

/// The relationships of one source part (or of the package itself).
#[derive(Debug, Clone, Default)]
pub struct Relationships {
    rels: Vec<Relationship>,
    original: Option<Vec<u8>>,
    dirty: bool,
}

impl Relationships {
    fn parse(bytes: &[u8]) -> Result<Relationships> {
        let doc = Document::parse(bytes)?;
        let mut rels = Vec::new();
        for el in doc.root.children_named(ns::PKG_RELS, "Relationship") {
            let get = |name: &str| {
                el.attr(name)
                    .map(str::to_string)
                    .ok_or_else(|| Error::Package(format!("relationship without {name}")))
            };
            rels.push(Relationship {
                id: get("Id")?,
                rel_type: get("Type")?,
                target: get("Target")?,
                external: el.attr("TargetMode") == Some("External"),
            });
        }
        Ok(Relationships {
            rels,
            original: Some(bytes.to_vec()),
            dirty: false,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.rels.iter()
    }

    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.rels.iter().find(|r| r.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.rels.is_empty()
    }

    pub fn remove(&mut self, id: &str) -> Option<Relationship> {
        let idx = self.rels.iter().position(|r| r.id == id)?;
        self.dirty = true;
        Some(self.rels.remove(idx))
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Relationship) -> bool) {
        let before = self.rels.len();
        self.rels.retain(|r| keep(r));
        if self.rels.len() != before {
            self.dirty = true;
        }
    }

    /// Adds a relationship with a fresh id and returns that id.
    pub fn add(&mut self, rel_type: &str, target: &str) -> String {
        let used: HashSet<&str> = self.rels.iter().map(|r| r.id.as_str()).collect();
        let id = (1..)
            .map(|n| format!("rId{n}"))
            .find(|id| !used.contains(id.as_str()))
            .expect("unbounded range");
        self.rels.push(Relationship {
            id: id.clone(),
            rel_type: rel_type.to_string(),
            target: target.to_string(),
            external: false,
        });
        self.dirty = true;
        id
    }

    /// Adds a relationship as is, keeping its id. Used when copying parts so
    /// `r:id` references in the copied XML stay valid.
    pub fn push(&mut self, rel: Relationship) {
        self.rels.retain(|r| r.id != rel.id);
        self.rels.push(rel);
        self.dirty = true;
    }

    pub fn set_target(&mut self, id: &str, target: &str) {
        if let Some(rel) = self.rels.iter_mut().find(|r| r.id == id) {
            rel.target = target.to_string();
            self.dirty = true;
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        if !self.dirty
            && let Some(original) = &self.original
        {
            return original.clone();
        }
        let mut out = String::from(concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            "\r\n",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#
        ));
        for r in &self.rels {
            let el = Element {
                name: "Relationship".into(),
                ns: None,
                attrs: [
                    Some(("Id", r.id.as_str())),
                    Some(("Type", r.rel_type.as_str())),
                    Some(("Target", r.target.as_str())),
                    r.external.then_some(("TargetMode", "External")),
                ]
                .into_iter()
                .flatten()
                .map(|(name, value)| Attr {
                    name: name.into(),
                    ns: None,
                    value: value.into(),
                })
                .collect(),
                children: Vec::new(),
            };
            let doc = Document {
                decl: None,
                prolog: Vec::new(),
                root: el,
                epilog: Vec::new(),
            };
            // Strip the declaration we get for free from Document::to_bytes.
            let bytes = doc.to_bytes();
            let s = String::from_utf8(bytes).expect("we only write UTF-8");
            out.push_str(s.split_once("?>\r\n").map_or(&s[..], |(_, body)| body));
        }
        out.push_str("</Relationships>");
        out.into_bytes()
    }
}

#[derive(Debug)]
pub struct Part {
    raw: Vec<u8>,
    xml: OnceLock<Document>,
    dirty: bool,
}

impl Part {
    fn new(raw: Vec<u8>) -> Part {
        Part {
            raw,
            xml: OnceLock::new(),
            dirty: false,
        }
    }

    fn from_xml(doc: Document) -> Part {
        let xml = OnceLock::new();
        let _ = xml.set(doc);
        Part {
            raw: Vec::new(),
            xml,
            dirty: true,
        }
    }

    fn xml(&self) -> Result<&Document> {
        if let Some(doc) = self.xml.get() {
            return Ok(doc);
        }
        let doc = Document::parse(&self.raw)?;
        Ok(self.xml.get_or_init(|| doc))
    }

    fn xml_mut(&mut self) -> Result<&mut Document> {
        self.xml()?;
        self.dirty = true;
        Ok(self.xml.get_mut().expect("initialised above"))
    }

    fn bytes(&self) -> Vec<u8> {
        match (self.dirty, self.xml.get()) {
            (true, Some(doc)) => doc.to_bytes(),
            _ => self.raw.clone(),
        }
    }
}

/// An OPC package held in memory.
#[derive(Debug)]
pub struct Package {
    parts: BTreeMap<String, Part>,
    /// Relationships keyed by source part name; `"/"` is the package itself.
    rels: BTreeMap<String, Relationships>,
    content_types: Document,
    /// Original bytes of `[Content_Types].xml`, written back while unchanged.
    content_types_raw: Option<Vec<u8>>,
    /// Zip entry order of the original file, used to write parts back in the
    /// same order.
    order: Vec<String>,
}

impl Package {
    pub fn from_reader<R: Read + Seek>(reader: R) -> Result<Package> {
        let mut zip = ZipArchive::new(reader)?;
        let mut parts = BTreeMap::new();
        let mut rels = BTreeMap::new();
        let mut content_types = None;
        let mut content_types_raw = None;
        let mut order = Vec::new();
        let mut total: u64 = 0;

        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            if file.is_dir() {
                continue;
            }
            let entry = file.name().to_string();
            let mut data = Vec::with_capacity(file.size().min(MAX_PART_SIZE) as usize);
            (&mut file).take(MAX_PART_SIZE + 1).read_to_end(&mut data)?;
            if data.len() as u64 > MAX_PART_SIZE {
                return Err(Error::Package(format!(
                    "{entry} is larger than {MAX_PART_SIZE} bytes"
                )));
            }
            total += data.len() as u64;
            if total > MAX_PACKAGE_SIZE {
                return Err(Error::Package(format!(
                    "package is larger than {MAX_PACKAGE_SIZE} bytes"
                )));
            }

            let name = format!("/{entry}");
            if entry.eq_ignore_ascii_case(CONTENT_TYPES) {
                content_types = Some(Document::parse(&data)?);
                content_types_raw = Some(data);
            } else if let Some(source) = rels_source(&name) {
                rels.insert(source, Relationships::parse(&data)?);
            } else {
                parts.insert(name.clone(), Part::new(data));
            }
            order.push(name);
        }

        let content_types =
            content_types.ok_or_else(|| Error::Package("missing [Content_Types].xml".into()))?;
        Ok(Package {
            parts,
            rels,
            content_types,
            content_types_raw,
            order,
        })
    }

    pub fn write<W: Write + Seek>(&mut self, writer: W) -> Result<W> {
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6));

        let mut written = HashSet::new();
        // Content types first, as Office does.
        zip.start_file(CONTENT_TYPES, options)?;
        match &self.content_types_raw {
            Some(raw) => zip.write_all(raw)?,
            None => zip.write_all(&self.content_types.to_bytes())?,
        }

        let mut names: Vec<String> = self.order.clone();
        for name in self.parts.keys() {
            names.push(name.clone());
        }
        for source in self.rels.keys() {
            names.push(rels_name(source));
        }
        for name in names {
            if !written.insert(name.clone()) {
                continue;
            }
            let bytes = if let Some(part) = self.parts.get(&name) {
                part.bytes()
            } else if let Some(source) = rels_source(&name) {
                match self.rels.get(&source) {
                    Some(r) if !r.is_empty() => r.to_bytes(),
                    _ => continue,
                }
            } else {
                // [Content_Types].xml or a part that was removed.
                continue;
            };
            zip.start_file(&name[1..], options)?;
            zip.write_all(&bytes)?;
        }
        Ok(zip.finish()?)
    }

    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        Ok(self.write(Cursor::new(Vec::new()))?.into_inner())
    }

    // ---- parts ------------------------------------------------------------

    pub fn has_part(&self, name: &str) -> bool {
        self.parts.contains_key(name)
    }

    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    pub fn xml(&self, name: &str) -> Result<&Document> {
        self.part(name)?.xml()
    }

    pub fn xml_mut(&mut self, name: &str) -> Result<&mut Document> {
        self.parts
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(Target::Part(name.into())))?
            .xml_mut()
    }

    pub fn raw(&self, name: &str) -> Result<Vec<u8>> {
        Ok(self.part(name)?.bytes())
    }

    fn part(&self, name: &str) -> Result<&Part> {
        self.parts
            .get(name)
            .ok_or_else(|| Error::NotFound(Target::Part(name.into())))
    }

    /// Replaces the raw content of a part, keeping its content type.
    pub fn set_raw(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        let part = self
            .parts
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(Target::Part(name.into())))?;
        *part = Part::new(data);
        Ok(())
    }

    pub fn add_xml_part(&mut self, name: &str, content_type: &str, doc: Document) {
        self.parts.insert(name.to_string(), Part::from_xml(doc));
        self.set_override(name, content_type);
    }

    pub fn add_raw_part(&mut self, name: &str, content_type: &str, data: Vec<u8>) {
        self.parts.insert(name.to_string(), Part::new(data));
        if self.content_type(name).as_deref() != Some(content_type) {
            self.set_override(name, content_type);
        }
    }

    /// Removes a part, its relationships and its content-type override.
    pub fn remove_part(&mut self, name: &str) {
        self.parts.remove(name);
        self.rels.remove(name);
        self.remove_override(name);
    }

    /// Returns an unused part name of the form `{prefix}{n}{suffix}`.
    pub fn next_part_name(&self, prefix: &str, suffix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}{suffix}"))
            .find(|name| {
                !self
                    .parts
                    .keys()
                    .any(|existing| existing.eq_ignore_ascii_case(name))
            })
            .expect("unbounded range")
    }

    // ---- content types ----------------------------------------------------

    pub fn content_type(&self, name: &str) -> Option<String> {
        let root = &self.content_types.root;
        let by_override = root
            .children_named(ns::CONTENT_TYPES, "Override")
            .find(|o| {
                o.attr("PartName")
                    .is_some_and(|p| p.eq_ignore_ascii_case(name))
            });
        if let Some(o) = by_override {
            return o.attr("ContentType").map(str::to_string);
        }
        let ext = name.rsplit_once('.')?.1;
        root.children_named(ns::CONTENT_TYPES, "Default")
            .find(|d| {
                d.attr("Extension")
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            })
            .and_then(|d| d.attr("ContentType"))
            .map(str::to_string)
    }

    fn set_override(&mut self, name: &str, content_type: &str) {
        self.remove_override(name);
        self.content_types_raw = None;
        let root = &mut self.content_types.root;
        let mut el = Element::new_like(root, "Override");
        el.set_attr("PartName", name);
        el.set_attr("ContentType", content_type);
        root.children.push(Node::Element(el));
    }

    fn remove_override(&mut self, name: &str) {
        let children = &mut self.content_types.root.children;
        let before = children.len();
        children.retain(|n| {
            !matches!(n, Node::Element(e)
                if e.is(ns::CONTENT_TYPES, "Override")
                    && e.attr("PartName").is_some_and(|p| p.eq_ignore_ascii_case(name)))
        });
        if children.len() != before {
            self.content_types_raw = None;
        }
    }

    // ---- relationships ----------------------------------------------------

    pub fn rels(&self, source: &str) -> Option<&Relationships> {
        self.rels.get(source)
    }

    pub fn rels_mut(&mut self, source: &str) -> &mut Relationships {
        self.rels
            .entry(source.to_string())
            .or_insert_with(|| Relationships {
                rels: Vec::new(),
                original: None,
                dirty: true,
            })
    }

    /// Absolute part name of the target of relationship `id` of `source`.
    pub fn resolve(&self, source: &str, id: &str) -> Result<String> {
        let rel = self
            .rels
            .get(source)
            .and_then(|r| r.get(id))
            .ok_or_else(|| Error::NotFound(Target::Relationship(format!("{source}#{id}"))))?;
        if rel.external {
            return Err(Error::Unsupported(format!(
                "relationship {id} of {source} points outside the package"
            )));
        }
        Ok(self.canonical(resolve_target(source, &rel.target)))
    }

    /// The stored name of a part. OPC part names are case-insensitive, and
    /// some producers write relationship targets in a different case than
    /// the zip entry.
    fn canonical(&self, name: String) -> String {
        if self.parts.contains_key(&name) {
            return name;
        }
        self.parts
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&name))
            .cloned()
            .unwrap_or(name)
    }

    /// Internal targets of all relationships of `source`, with their types.
    pub fn targets(&self, source: &str) -> Vec<(Relationship, String)> {
        self.rels
            .get(source)
            .into_iter()
            .flat_map(|r| r.iter())
            .filter(|r| !r.external)
            .map(|r| (r.clone(), self.canonical(resolve_target(source, &r.target))))
            .collect()
    }

    /// Drops every part that can no longer be reached from the package
    /// relationships. Returns the removed part names.
    pub fn collect_garbage(&mut self) -> Vec<String> {
        let mut reachable = HashSet::new();
        let mut queue = VecDeque::from(["/".to_string()]);
        while let Some(source) = queue.pop_front() {
            for (_, target) in self.targets(&source) {
                if self.parts.contains_key(&target) && reachable.insert(target.clone()) {
                    queue.push_back(target);
                }
            }
        }
        let garbage: Vec<String> = self
            .parts
            .keys()
            .filter(|name| !reachable.contains(*name))
            .cloned()
            .collect();
        for name in &garbage {
            self.remove_part(name);
        }
        self.rels
            .retain(|source, _| source == "/" || self.parts.contains_key(source));
        garbage
    }
}

/// `/ppt/slides/_rels/slide1.xml.rels` -> `/ppt/slides/slide1.xml`, `/_rels/.rels` -> `/`.
fn rels_source(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".rels")?;
    let (dir, file) = stem.rsplit_once('/')?;
    let dir = dir
        .strip_suffix("/_rels")
        .or_else(|| (dir == "/_rels").then_some(""))?;
    if file.is_empty() {
        return Some("/".to_string());
    }
    Some(format!("{dir}/{file}"))
}

fn rels_name(source: &str) -> String {
    if source == "/" {
        return "/_rels/.rels".to_string();
    }
    let (dir, file) = source.rsplit_once('/').unwrap_or(("", source));
    format!("{dir}/_rels/{file}.rels")
}

/// Resolves a relationship target (a relative URI, possibly percent-encoded)
/// against its source part.
pub fn resolve_target(source: &str, target: &str) -> String {
    let target = percent_decode(target);
    if target.starts_with('/') {
        return normalize(&target);
    }
    let base = source.rsplit_once('/').map_or("", |(dir, _)| dir);
    normalize(&format!("{base}/{target}"))
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encodes a part name for use as a relationship target.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn normalize(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    format!("/{}", segments.join("/"))
}

/// Relative path from the directory of `source` to `target` (both absolute).
pub fn relative_target(source: &str, target: &str) -> String {
    let from: Vec<&str> = source
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    percent_encode(&parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_names() {
        assert_eq!(rels_source("/_rels/.rels").as_deref(), Some("/"));
        assert_eq!(
            rels_source("/ppt/slides/_rels/slide1.xml.rels").as_deref(),
            Some("/ppt/slides/slide1.xml")
        );
        assert_eq!(rels_source("/ppt/slides/slide1.xml"), None);
        assert_eq!(rels_name("/"), "/_rels/.rels");
        assert_eq!(
            rels_name("/ppt/slides/slide1.xml"),
            "/ppt/slides/_rels/slide1.xml.rels"
        );
    }

    #[test]
    fn targets() {
        assert_eq!(
            resolve_target("/ppt/slides/slide1.xml", "../charts/chart1.xml"),
            "/ppt/charts/chart1.xml"
        );
        assert_eq!(
            resolve_target("/", "ppt/presentation.xml"),
            "/ppt/presentation.xml"
        );
        assert_eq!(
            resolve_target("/ppt/slides/slide1.xml", "/ppt/media/a.png"),
            "/ppt/media/a.png"
        );
        assert_eq!(
            relative_target("/ppt/slides/slide1.xml", "/ppt/charts/chart1.xml"),
            "../charts/chart1.xml"
        );
        assert_eq!(
            relative_target("/ppt/presentation.xml", "/ppt/slides/slide2.xml"),
            "slides/slide2.xml"
        );
        assert_eq!(
            resolve_target("/ppt/slides/slide1.xml", "../media/image%201%C3%BC.png"),
            "/ppt/media/image 1ü.png"
        );
        assert_eq!(
            relative_target("/ppt/slides/slide1.xml", "/ppt/media/image 1ü.png"),
            "../media/image%201%C3%BC.png"
        );
        assert_eq!(percent_decode("100%"), "100%");
    }
}
