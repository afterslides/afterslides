//! Reading and writing DrawingML text bodies (`a:txBody`, `c:rich`, ...).
//!
//! Conventions follow python-pptx so code ported from there keeps working:
//! paragraphs are separated by `\n`, line breaks (`a:br`) inside a paragraph
//! are `\v`.

use crate::xml::{Element, Node, ns};

pub const PARAGRAPH_SEP: char = '\n';
pub const LINE_BREAK: char = '\u{b}';

/// Plain text of a text body.
pub fn get_text(body: &Element) -> String {
    let mut out = String::new();
    for (i, p) in body.children_named(ns::A, "p").enumerate() {
        if i > 0 {
            out.push(PARAGRAPH_SEP);
        }
        out.push_str(&paragraph_text(p));
    }
    out
}

pub fn paragraph_text(p: &Element) -> String {
    let mut out = String::new();
    for e in p.elements() {
        match (e.ns(), e.local()) {
            (Some(ns::A), "r" | "fld") => {
                if let Some(t) = e.child(ns::A, "t") {
                    out.push_str(&t.text());
                }
            }
            (Some(ns::A), "br") => out.push(LINE_BREAK),
            _ => {}
        }
    }
    out
}

/// Replaces the text of a body, keeping the formatting of the first
/// paragraph and its first run.
pub fn set_text(body: &mut Element, text: &str) {
    let template_p = body.child(ns::A, "p").cloned();
    let (p_pr, r_pr, end_pr) = match &template_p {
        Some(p) => (
            p.child(ns::A, "pPr").cloned(),
            first_run_properties(p),
            p.child(ns::A, "endParaRPr").cloned(),
        ),
        None => (None, None, None),
    };

    let first_p = body.position(ns::A, "p");
    body.remove_children(ns::A, "p");
    let insert_at = first_p.unwrap_or(body.children.len());

    let paragraphs: Vec<Node> = text
        .split(PARAGRAPH_SEP)
        .map(|line| {
            let mut p = Element::new_like(body_like_a(body), "p");
            if let Some(ppr) = &p_pr {
                p.children.push(Node::Element(ppr.clone()));
            }
            for (i, segment) in line.split(LINE_BREAK).enumerate() {
                if i > 0 {
                    let mut br = Element::new_like(&p, "br");
                    if let Some(rpr) = &r_pr {
                        br.children.push(Node::Element(rpr.clone()));
                    }
                    p.children.push(Node::Element(br));
                }
                if !segment.is_empty() {
                    p.children
                        .push(Node::Element(make_run(&p, r_pr.as_ref(), segment)));
                }
            }
            if let Some(end) = end_pr.clone().or_else(|| {
                r_pr.clone().map(|mut rpr| {
                    rpr.name = rpr.name.replace("rPr", "endParaRPr");
                    rpr.remove_descendants(&|_| true);
                    rpr
                })
            }) {
                p.children.push(Node::Element(end));
            }
            Node::Element(p)
        })
        .collect();

    body.children.splice(insert_at..insert_at, paragraphs);
}

/// Text bodies in charts (`c:rich`) live in the chart namespace but their
/// paragraphs are DrawingML; make sure new paragraphs get the `a:` prefix.
fn body_like_a(body: &Element) -> &Element {
    body.elements()
        .find(|e| e.ns() == Some(ns::A))
        .unwrap_or(body)
}

fn first_run_properties(p: &Element) -> Option<Element> {
    p.elements()
        .filter(|e| e.is(ns::A, "r") || e.is(ns::A, "fld"))
        .find_map(|r| r.child(ns::A, "rPr").cloned())
        .or_else(|| {
            p.child(ns::A, "endParaRPr").map(|end| {
                let mut rpr = end.clone();
                rpr.name = rpr.name.replace("endParaRPr", "rPr");
                rpr
            })
        })
}

fn make_run(like: &Element, rpr: Option<&Element>, text: &str) -> Element {
    let mut r = Element::new_like(like, "r");
    if let Some(rpr) = rpr {
        r.children.push(Node::Element(rpr.clone()));
    }
    let mut t = Element::new_like(like, "t");
    t.set_text(text);
    r.children.push(Node::Element(t));
    r
}

/// Replaces every occurrence of each key with its value inside `root`,
/// including occurrences that span several runs. The replacement takes the
/// formatting of the run in which the match starts.
///
/// Returns the number of replacements made.
pub fn replace_in(root: &mut Element, replacements: &[(&str, &str)]) -> usize {
    let replacements: Vec<(&str, &str)> = replacements
        .iter()
        .copied()
        .filter(|(from, _)| !from.is_empty())
        .collect();
    if replacements.is_empty() {
        return 0;
    }
    let mut count = 0;
    root.walk_mut(&mut |el| {
        if el.is(ns::A, "p") {
            count += replace_in_paragraph(el, &replacements);
        }
    });
    count
}

fn replace_in_paragraph(p: &mut Element, replacements: &[(&str, &str)]) -> usize {
    // Runs separated by anything else (line breaks, fields) are matched
    // independently.
    let mut groups: Vec<Vec<usize>> = vec![Vec::new()];
    for (i, child) in p.children.iter().enumerate() {
        match child {
            Node::Element(e) if e.is(ns::A, "r") => groups.last_mut().unwrap().push(i),
            Node::Element(_) => groups.push(Vec::new()),
            _ => {}
        }
    }

    let mut count = 0;
    let mut emptied: Vec<usize> = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        let texts: Vec<String> = group.iter().map(|&i| run_text(&p.children[i])).collect();
        let (new_texts, n) = replace_across(&texts, replacements);
        if n == 0 {
            continue;
        }
        count += n;
        for ((&idx, old), new) in group.iter().zip(&texts).zip(new_texts) {
            if &new == old {
                continue;
            }
            if new.is_empty() {
                emptied.push(idx);
            } else if let Node::Element(r) = &mut p.children[idx] {
                set_run_text(r, &new);
            }
        }
    }
    for idx in emptied.into_iter().rev() {
        p.children.remove(idx);
    }
    count
}

fn run_text(node: &Node) -> String {
    match node {
        Node::Element(r) => r.child(ns::A, "t").map(Element::text).unwrap_or_default(),
        _ => String::new(),
    }
}

fn set_run_text(r: &mut Element, text: &str) {
    match r.child_mut(ns::A, "t") {
        Some(t) => t.set_text(text),
        None => {
            let mut t = Element::new_like(r, "t");
            t.set_text(text);
            r.children.push(Node::Element(t));
        }
    }
}

/// Core of the cross-run replacement, on plain strings so it can be tested
/// in isolation. Returns the new text of each run and the match count.
fn replace_across(runs: &[String], replacements: &[(&str, &str)]) -> (Vec<String>, usize) {
    let joined: String = runs.concat();
    let mut matches: Vec<(usize, usize, &str)> = Vec::new();
    let mut pos = 0;
    while pos < joined.len() {
        let hay = &joined[pos..];
        // Earliest match wins; on a tie the longest key wins.
        let best = replacements
            .iter()
            .filter_map(|(from, to)| hay.find(from).map(|at| (at, from.len(), *to)))
            .min_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        match best {
            Some((at, len, to)) => {
                matches.push((pos + at, pos + at + len, to));
                pos += at + len;
            }
            None => break,
        }
    }
    if matches.is_empty() {
        return (runs.to_vec(), 0);
    }

    let mut out = Vec::with_capacity(runs.len());
    let mut run_start = 0;
    for run in runs {
        let run_end = run_start + run.len();
        let mut text = String::new();
        let mut cursor = run_start;
        for &(m_start, m_end, to) in &matches {
            if m_end <= run_start || m_start >= run_end {
                continue;
            }
            if m_start > cursor {
                text.push_str(&joined[cursor..m_start]);
            }
            if m_start >= run_start {
                text.push_str(to);
            }
            cursor = m_end.min(run_end);
        }
        if cursor < run_end {
            text.push_str(&joined[cursor..run_end]);
        }
        out.push(text);
        run_start = run_end;
    }
    (out, matches.len())
}

/// Creates a minimal `a:txBody`-like element (`p:txBody` for shapes, `a:txBody`
/// for table cells) with one empty paragraph.
pub fn new_body(name_like: &Element, local: &str, a_like: &Element) -> Element {
    let mut body = Element::new_like(name_like, local);
    for child in ["bodyPr", "lstStyle", "p"] {
        body.children
            .push(Node::Element(Element::new_like(a_like, child)));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Document;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn replaces_within_one_run() {
        let (out, n) = replace_across(&s(&["Hello {{name}}!"]), &[("{{name}}", "Ada")]);
        assert_eq!(out, s(&["Hello Ada!"]));
        assert_eq!(n, 1);
    }

    #[test]
    fn replaces_across_runs() {
        let (out, n) = replace_across(
            &s(&["Hello {{na", "me", "}}! and {{name}}"]),
            &[("{{name}}", "Ada")],
        );
        assert_eq!(out, s(&["Hello Ada", "", "! and Ada"]));
        assert_eq!(n, 2);
    }

    #[test]
    fn longest_key_wins() {
        let (out, _) = replace_across(&s(&["{{a}}{{ab}}"]), &[("{{a", "X"), ("{{ab}}", "Y")]);
        assert_eq!(out, s(&["X}}Y"]));
    }

    #[test]
    fn unicode_boundaries() {
        let (out, _) = replace_across(&s(&["ä{{", "ö}}ü"]), &[("{{ö}}", "→")]);
        assert_eq!(out, s(&["ä→", "ü"]));
    }

    const BODY: &str = r#"<p:txBody xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:bodyPr/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" b="1"/><a:t>Revenue {{</a:t></a:r><a:r><a:rPr lang="en-US"/><a:t>year}}</a:t></a:r><a:br><a:rPr lang="en-US"/></a:br><a:r><a:rPr lang="en-US"/><a:t>second line</a:t></a:r><a:endParaRPr lang="en-US"/></a:p><a:p><a:r><a:t>para 2</a:t></a:r></a:p></p:txBody>"#;

    #[test]
    fn get_text_uses_python_pptx_conventions() {
        let doc = Document::parse(BODY.as_bytes()).unwrap();
        assert_eq!(
            get_text(&doc.root),
            "Revenue {{year}}\u{b}second line\npara 2"
        );
    }

    #[test]
    fn replace_in_body_removes_emptied_runs() {
        let mut doc = Document::parse(BODY.as_bytes()).unwrap();
        let n = replace_in(&mut doc.root, &[("{{year}}", "2026")]);
        assert_eq!(n, 1);
        assert_eq!(get_text(&doc.root), "Revenue 2026\u{b}second line\npara 2");
        let p = doc.root.child(ns::A, "p").unwrap();
        assert_eq!(p.children_named(ns::A, "r").count(), 2);
        // The replacement keeps the bold formatting of the run it started in.
        let xml = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(xml.contains(r#"<a:rPr lang="en-US" b="1"/><a:t>Revenue 2026</a:t>"#));
    }

    #[test]
    fn set_text_keeps_formatting() {
        let mut doc = Document::parse(BODY.as_bytes()).unwrap();
        set_text(&mut doc.root, "one\ntwo\u{b}three");
        assert_eq!(get_text(&doc.root), "one\ntwo\u{b}three");
        let xml = String::from_utf8(doc.to_bytes()).unwrap();
        assert_eq!(xml.matches(r#"<a:pPr algn="ctr"/>"#).count(), 2);
        assert!(xml.contains(r#"<a:r><a:rPr lang="en-US" b="1"/><a:t>one</a:t></a:r>"#));
        // bodyPr and lstStyle stay in front of the paragraphs.
        let names: Vec<_> = doc.root.elements().map(|e| e.local().to_string()).collect();
        assert_eq!(names, ["bodyPr", "lstStyle", "p", "p"]);
    }

    #[test]
    fn set_empty_text() {
        let mut doc = Document::parse(BODY.as_bytes()).unwrap();
        set_text(&mut doc.root, "");
        assert_eq!(get_text(&doc.root), "");
        assert_eq!(doc.root.children_named(ns::A, "p").count(), 1);
    }
}
