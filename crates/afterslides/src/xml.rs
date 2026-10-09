//! A small mutable XML tree.
//!
//! OOXML parts are edited in place and written back, so the tree keeps
//! everything a template author might rely on: qualified names exactly as
//! written (PowerPoint refers to prefixes by name in `mc:Ignorable`), attribute
//! order, comments and processing instructions. Every element and attribute
//! also carries its resolved namespace URI so lookups don't depend on the
//! prefix a producer happened to choose.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::error::{Error, Result};

pub mod ns {
    pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    pub const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
    pub const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    pub const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
    pub const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
    pub const XML: &str = "http://www.w3.org/XML/1998/namespace";
    pub const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
    ProcessingInstruction(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    /// Qualified name as written, e.g. `r:id`.
    pub name: String,
    /// Namespace of the attribute; `None` for unprefixed attributes.
    pub ns: Option<Arc<str>>,
    pub value: String,
}

impl Attr {
    pub fn local(&self) -> &str {
        local_part(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Qualified name as written, e.g. `a:t`.
    pub name: String,
    pub ns: Option<Arc<str>>,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

fn local_part(qname: &str) -> &str {
    qname.rsplit_once(':').map_or(qname, |(_, local)| local)
}

fn prefix_part(qname: &str) -> Option<&str> {
    qname.split_once(':').map(|(prefix, _)| prefix)
}

impl Element {
    /// Creates an element with the same prefix and namespace as `like`.
    ///
    /// New elements are almost always siblings or children of existing ones in
    /// the same namespace, so borrowing the prefix keeps the output consistent
    /// with whatever the template uses.
    pub fn new_like(like: &Element, local: &str) -> Element {
        let name = match prefix_part(&like.name) {
            Some(prefix) => format!("{prefix}:{local}"),
            None => local.to_string(),
        };
        Element {
            name,
            ns: like.ns.clone(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn local(&self) -> &str {
        local_part(&self.name)
    }

    pub fn ns(&self) -> Option<&str> {
        self.ns.as_deref()
    }

    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.local() == local && self.ns() == Some(ns)
    }

    // ---- attributes -------------------------------------------------------

    /// Value of an unprefixed attribute.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    /// Value of a namespaced attribute, e.g. `(ns::R, "id")`.
    pub fn attr_ns(&self, ns: &str, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.as_deref() == Some(ns) && a.local() == local)
            .map(|a| a.value.as_str())
    }

    /// Sets an unprefixed attribute, appending it if it does not exist yet.
    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .attrs
            .iter_mut()
            .find(|a| a.ns.is_none() && a.name == name)
        {
            Some(attr) => attr.value = value,
            None => self.attrs.push(Attr {
                name: name.to_string(),
                ns: None,
                value,
            }),
        }
    }

    /// Sets the value of an existing namespaced attribute. Returns `false` if
    /// the attribute is not present; creating one would need a prefix binding.
    pub fn set_attr_ns(&mut self, ns: &str, local: &str, value: impl Into<String>) -> bool {
        match self
            .attrs
            .iter_mut()
            .find(|a| a.ns.as_deref() == Some(ns) && a.local() == local)
        {
            Some(attr) => {
                attr.value = value.into();
                true
            }
            None => false,
        }
    }

    pub fn remove_attr_ns(&mut self, ns: &str, local: &str) {
        self.attrs
            .retain(|a| !(a.ns.as_deref() == Some(ns) && a.local() == local));
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|a| !(a.ns.is_none() && a.name == name));
    }

    // ---- children ---------------------------------------------------------

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, ns: &str, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, local))
    }

    pub fn child_mut(&mut self, ns: &str, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.is(ns, local))
    }

    pub fn children_named<'a>(
        &'a self,
        ns: &'a str,
        local: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.is(ns, local))
    }

    pub fn children_named_mut<'a>(
        &'a mut self,
        ns: &'a str,
        local: &'a str,
    ) -> impl Iterator<Item = &'a mut Element> + 'a {
        self.elements_mut().filter(move |e| e.is(ns, local))
    }

    /// Follows a path of `(ns, local)` steps, taking the first match each time.
    pub fn path(&self, steps: &[(&str, &str)]) -> Option<&Element> {
        let mut cur = self;
        for (ns, local) in steps {
            cur = cur.child(ns, local)?;
        }
        Some(cur)
    }

    pub fn path_mut(&mut self, steps: &[(&str, &str)]) -> Option<&mut Element> {
        let mut cur = self;
        for (ns, local) in steps {
            cur = cur.child_mut(ns, local)?;
        }
        Some(cur)
    }

    /// Index into `children` of the first child element matching the name.
    pub fn position(&self, ns: &str, local: &str) -> Option<usize> {
        self.children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if e.is(ns, local)))
    }

    /// Removes all child elements matching the name. Returns how many were removed.
    pub fn remove_children(&mut self, ns: &str, local: &str) -> usize {
        let before = self.children.len();
        self.children
            .retain(|n| !matches!(n, Node::Element(e) if e.is(ns, local)));
        before - self.children.len()
    }

    /// Returns the child element with the given name, creating it if needed.
    ///
    /// `after` lists sibling names (same namespace) that must precede the new
    /// element according to the schema; it is inserted after the last of them
    /// that is present, or first if none are.
    pub fn ensure_child(&mut self, local: &str, after: &[&str]) -> &mut Element {
        let ns = self.ns.clone();
        let ns_str = ns.as_deref().unwrap_or_default();
        if let Some(idx) = self.position(ns_str, local) {
            return match &mut self.children[idx] {
                Node::Element(e) => e,
                _ => unreachable!(),
            };
        }
        let insert_at = self
            .children
            .iter()
            .rposition(|n| {
                matches!(n, Node::Element(e)
                    if e.ns() == Some(ns_str) && after.contains(&e.local()))
            })
            .map_or(0, |i| i + 1);
        let new = Element::new_like(self, local);
        self.children.insert(insert_at, Node::Element(new));
        match &mut self.children[insert_at] {
            Node::Element(e) => e,
            _ => unreachable!(),
        }
    }

    /// Concatenated text of all descendant text nodes.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                Node::Text(t) | Node::CData(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
                _ => {}
            }
        }
    }

    /// Replaces all children with a single text node.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.children = vec![Node::Text(text.into())];
    }

    /// Depth-first walk over this element and all descendants.
    pub fn walk(&self, f: &mut impl FnMut(&Element)) {
        f(self);
        for e in self.elements() {
            e.walk(f);
        }
    }

    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Element)) {
        f(self);
        for e in self.elements_mut() {
            e.walk_mut(f);
        }
    }

    /// Removes descendants (not `self`) for which `pred` returns true.
    pub fn remove_descendants(&mut self, pred: &impl Fn(&Element) -> bool) -> usize {
        let mut removed = 0;
        let before = self.children.len();
        self.children
            .retain(|n| !matches!(n, Node::Element(e) if pred(e)));
        removed += before - self.children.len();
        for e in self.elements_mut() {
            removed += e.remove_descendants(pred);
        }
        removed
    }
}

// ---- documents ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Raw content of the XML declaration, e.g. `version="1.0" encoding="UTF-8"`.
    pub decl: Option<String>,
    /// Comments and processing instructions before the root element.
    pub prolog: Vec<Node>,
    pub root: Element,
    /// Comments and processing instructions after the root element.
    pub epilog: Vec<Node>,
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Document> {
        let text = decode(bytes)?;
        Parser::default().parse(&text)
    }

    /// Prefix bound to `uri` on the root element, if any.
    pub fn prefix_for(&self, uri: &str) -> Option<&str> {
        self.root.attrs.iter().find_map(|a| {
            (a.value == uri)
                .then(|| a.name.strip_prefix("xmlns:"))
                .flatten()
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(4096);
        match &self.decl {
            Some(decl) => {
                out.push_str("<?xml ");
                out.push_str(decl);
                out.push_str("?>");
            }
            None => out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#),
        }
        // PowerPoint writes CRLF after the declaration; keep that convention.
        out.push_str("\r\n");
        for node in &self.prolog {
            write_node(node, &mut out);
        }
        write_element(&self.root, &mut out);
        for node in &self.epilog {
            write_node(node, &mut out);
        }
        out.into_bytes()
    }
}

fn decode(bytes: &[u8]) -> Result<Cow<'_, str>> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(rest)
            .map(Cow::Borrowed)
            .map_err(|e| Error::Xml(format!("invalid UTF-8: {e}")));
    }
    let utf16 = |le: bool, data: &[u8]| -> Result<Cow<'_, str>> {
        let units = data.as_chunks::<2>().0.iter().map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        });
        let decoded: String = char::decode_utf16(units)
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| Error::Xml(format!("invalid UTF-16: {e}")))?;
        // The declaration still says UTF-16; it is rewritten on save.
        Ok(Cow::Owned(decoded))
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(true, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(false, rest),
        _ => std::str::from_utf8(bytes)
            .map(Cow::Borrowed)
            .map_err(|e| Error::Xml(format!("invalid UTF-8: {e}"))),
    }
}

#[derive(Default)]
struct Parser {
    /// Interned namespace URIs, so thousands of elements share one allocation.
    interned: HashMap<String, Arc<str>>,
    /// Stack of prefix bindings; `""` is the default namespace.
    scopes: Vec<Vec<(String, Arc<str>)>>,
}

impl Parser {
    fn intern(&mut self, uri: &str) -> Arc<str> {
        if let Some(arc) = self.interned.get(uri) {
            return arc.clone();
        }
        let arc: Arc<str> = Arc::from(uri);
        self.interned.insert(uri.to_string(), arc.clone());
        arc
    }

    fn lookup(&self, prefix: &str) -> Option<Arc<str>> {
        match prefix {
            "xml" => return Some(Arc::from(ns::XML)),
            "xmlns" => return Some(Arc::from(ns::XMLNS)),
            _ => {}
        }
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|(p, _)| p == prefix)
            .map(|(_, uri)| uri.clone())
    }

    fn start(&mut self, start: &BytesStart<'_>) -> Result<Element> {
        let name = start.name().as_ref().to_string();
        let mut raw_attrs = Vec::new();
        let mut scope = Vec::new();
        for attr in start.attributes() {
            let attr = attr.map_err(|e| Error::Xml(e.to_string()))?;
            let key = attr.key.as_ref().to_string();
            let value = attr
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|e| Error::Xml(e.to_string()))?
                .into_owned();
            if key == "xmlns" {
                scope.push((String::new(), self.intern(&value)));
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                scope.push((prefix.to_string(), self.intern(&value)));
            }
            raw_attrs.push((key, value));
        }
        self.scopes.push(scope);

        let ns = self.lookup(prefix_part(&name).unwrap_or(""));
        let attrs = raw_attrs
            .into_iter()
            .map(|(name, value)| {
                let ns = if name == "xmlns" {
                    Some(Arc::from(ns::XMLNS))
                } else {
                    prefix_part(&name).and_then(|p| self.lookup(p))
                };
                Attr { name, ns, value }
            })
            .collect();
        Ok(Element {
            name,
            ns,
            attrs,
            children: Vec::new(),
        })
    }

    fn parse(mut self, text: &str) -> Result<Document> {
        let mut reader = Reader::from_str(text);
        let config = reader.config_mut();
        config.trim_text(false);
        config.check_end_names = true;

        let mut decl = None;
        let mut prolog = Vec::new();
        let mut epilog = Vec::new();
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;

        let push_node = |stack: &mut Vec<Element>,
                         root: &Option<Element>,
                         prolog: &mut Vec<Node>,
                         epilog: &mut Vec<Node>,
                         node: Node| {
            if let Some(parent) = stack.last_mut() {
                // Merge adjacent text so entity references don't fragment runs.
                if let (Node::Text(new), Some(Node::Text(prev))) =
                    (&node, parent.children.last_mut())
                {
                    prev.push_str(new);
                    return;
                }
                parent.children.push(node);
            } else if root.is_some() {
                if !matches!(node, Node::Text(_)) {
                    epilog.push(node);
                }
            } else if !matches!(node, Node::Text(_)) {
                prolog.push(node);
            }
        };

        loop {
            let event = reader
                .read_event()
                .map_err(|e| Error::Xml(format!("at byte {}: {e}", reader.buffer_position())))?;
            match event {
                Event::Decl(d) => {
                    decl = Some(d.trim_start_matches("xml").trim().to_string());
                }
                Event::Start(s) => {
                    let el = self.start(&s)?;
                    stack.push(el);
                }
                Event::Empty(s) => {
                    let el = self.start(&s)?;
                    self.scopes.pop();
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(Node::Element(el)),
                        None => root = Some(el),
                    }
                }
                Event::End(_) => {
                    self.scopes.pop();
                    let el = stack
                        .pop()
                        .ok_or_else(|| Error::Xml("unbalanced end tag".into()))?;
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(Node::Element(el)),
                        None => root = Some(el),
                    }
                }
                Event::Text(t) => {
                    let s = t.xml10_content().into_owned();
                    push_node(&mut stack, &root, &mut prolog, &mut epilog, Node::Text(s));
                }
                Event::GeneralRef(r) => {
                    let resolved = match r.resolve_char_ref() {
                        Ok(Some(ch)) => ch.to_string(),
                        Ok(None) => {
                            let name: &str = &r;
                            quick_xml::escape::resolve_predefined_entity(name)
                                .ok_or_else(|| Error::Xml(format!("unknown entity &{name};")))?
                                .to_string()
                        }
                        Err(e) => return Err(Error::Xml(e.to_string())),
                    };
                    push_node(
                        &mut stack,
                        &root,
                        &mut prolog,
                        &mut epilog,
                        Node::Text(resolved),
                    );
                }
                Event::CData(c) => {
                    let s = c.to_string();
                    push_node(&mut stack, &root, &mut prolog, &mut epilog, Node::CData(s));
                }
                Event::Comment(c) => {
                    let s: &str = &c;
                    push_node(
                        &mut stack,
                        &root,
                        &mut prolog,
                        &mut epilog,
                        Node::Comment(s.to_string()),
                    );
                }
                Event::PI(p) => {
                    let s = p.to_string();
                    push_node(
                        &mut stack,
                        &root,
                        &mut prolog,
                        &mut epilog,
                        Node::ProcessingInstruction(s),
                    );
                }
                // DTDs have no place in OOXML; dropping them also means we never
                // expand user-defined entities.
                Event::DocType(_) => {}
                Event::Eof => break,
            }
        }
        if !stack.is_empty() {
            return Err(Error::Xml("unexpected end of document".into()));
        }
        let root = root.ok_or_else(|| Error::Xml("document has no root element".into()))?;
        Ok(Document {
            decl: decl.map(|d| {
                // We always write UTF-8.
                d.replace("UTF-16", "UTF-8").replace("utf-16", "UTF-8")
            }),
            prolog,
            root,
            epilog,
        })
    }
}

fn write_node(node: &Node, out: &mut String) {
    match node {
        Node::Element(e) => write_element(e, out),
        Node::Text(t) => escape_into(t, out, false),
        Node::CData(t) => {
            out.push_str("<![CDATA[");
            out.push_str(t);
            out.push_str("]]>");
        }
        Node::Comment(t) => {
            out.push_str("<!--");
            out.push_str(t);
            out.push_str("-->");
        }
        Node::ProcessingInstruction(t) => {
            out.push_str("<?");
            out.push_str(t);
            out.push_str("?>");
        }
    }
}

fn write_element(e: &Element, out: &mut String) {
    out.push('<');
    out.push_str(&e.name);
    for a in &e.attrs {
        out.push(' ');
        out.push_str(&a.name);
        out.push_str("=\"");
        escape_into(&a.value, out, true);
        out.push('"');
    }
    if e.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &e.children {
        write_node(child, out);
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

fn escape_into(s: &str, out: &mut String, attr: bool) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            // Literal whitespace in attributes would be normalized to spaces
            // when read back.
            '\n' if attr => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attr => out.push_str("&#9;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        "\r\n",
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
        r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" "#,
        r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="p14">"#,
        r#"<p:cSld><!-- keep me --><a:t xml:space="preserve"> a &amp; b &lt;c&gt; &#x263A; </a:t>"#,
        r#"<x:other xmlns:x="urn:x" x:val="1&quot;"/></p:cSld></p:sld>"#,
    );

    #[test]
    fn round_trip_preserves_names_and_text() {
        let doc = Document::parse(SAMPLE.as_bytes()).unwrap();
        let out = String::from_utf8(doc.to_bytes()).unwrap();
        let expected = SAMPLE.replace("&#x263A;", "\u{263A}");
        assert_eq!(out, expected);
    }

    #[test]
    fn resolves_namespaces_independent_of_prefix() {
        let xml = r#"<root xmlns="http://schemas.openxmlformats.org/drawingml/2006/main"><t>x</t></root>"#;
        let doc = Document::parse(xml.as_bytes()).unwrap();
        let t = doc.root.child(ns::A, "t").unwrap();
        assert_eq!(t.text(), "x");
        let other = Document::parse(SAMPLE.as_bytes()).unwrap();
        let x = other
            .root
            .path(&[(ns::P, "cSld"), ("urn:x", "other")])
            .unwrap();
        assert_eq!(x.attr_ns("urn:x", "val"), Some("1\""));
    }

    #[test]
    fn ensure_child_respects_order() {
        let xml = r#"<c:ser xmlns:c="urn:c"><c:idx val="0"/><c:tx/><c:val/></c:ser>"#;
        let mut doc = Document::parse(xml.as_bytes()).unwrap();
        doc.root
            .ensure_child("cat", &["idx", "order", "tx", "spPr"]);
        let names: Vec<_> = doc.root.elements().map(|e| e.local().to_string()).collect();
        assert_eq!(names, ["idx", "tx", "cat", "val"]);
    }

    #[test]
    fn rejects_unknown_entities() {
        let xml = r#"<!DOCTYPE r [<!ENTITY e "boom">]><r>&e;</r>"#;
        assert!(Document::parse(xml.as_bytes()).is_err());
    }

    #[test]
    fn utf16_input() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><r>ü</r>"#;
        let mut bytes = vec![0xFF, 0xFE];
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let doc = Document::parse(&bytes).unwrap();
        assert_eq!(doc.root.text(), "ü");
        assert!(String::from_utf8(doc.to_bytes()).unwrap().contains("UTF-8"));
    }
}
