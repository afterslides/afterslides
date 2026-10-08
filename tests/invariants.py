"""Structural checks for saved decks.

python-pptx and LibreOffice are lenient readers: they open files PowerPoint
would want to repair. These checks encode the rules PowerPoint is strict
about, and run on every deck the test suite produces.
"""

from __future__ import annotations

import io
import posixpath
import re
import zipfile
from collections import Counter
from urllib.parse import unquote

from lxml import etree

R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
P14 = "http://schemas.microsoft.com/office/powerpoint/2010/main"
PKG_RELS = "http://schemas.openxmlformats.org/package/2006/relationships"
CT = "http://schemas.openxmlformats.org/package/2006/content-types"

# Children of p:sp in schema order.
SP_ORDER = ["nvSpPr", "spPr", "style", "txBody", "extLst"]


class InvariantError(AssertionError):
    pass


def _rels_name(part: str) -> str:
    d, f = posixpath.split(part)
    return posixpath.join(d, "_rels", f + ".rels")


def _resolve(source: str, target: str) -> str:
    target = unquote(target)
    if target.startswith("/"):
        return posixpath.normpath(target)
    return posixpath.normpath(posixpath.join(posixpath.dirname(source), target))


def check_deck(data: bytes, baseline: bytes | None = None) -> None:
    """Raises InvariantError listing every violation found.

    With a baseline (the deck before editing), only kinds of problems the
    input didn't already have are reported: real-world files come with
    their own defects, and we only answer for the ones we introduce.
    """
    found = problems(data)
    if baseline is not None:
        known = {_kind(p) for p in problems(baseline)}
        found = [p for p in found if _kind(p) not in known]
    if found:
        raise InvariantError("deck violates invariants:\n  " + "\n  ".join(found))


def _kind(problem: str) -> str:
    return re.sub(r"\d+", "#", problem)


def problems(data: bytes) -> list[str]:
    problems: list[str] = []
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        names = [n for n in z.namelist() if not n.endswith("/")]
        parts = {"/" + n: z.read(n) for n in names}
    lower = {unquote(name).lower(): name for name in parts}

    def exists(name: str) -> bool:
        return unquote(name).lower() in lower

    # Content types: every part has one, every override has a part.
    ct = etree.fromstring(parts["/[Content_Types].xml"])
    defaults = {
        d.get("Extension").lower(): d.get("ContentType") for d in ct.findall(f"{{{CT}}}Default")
    }
    overrides = {
        o.get("PartName").lower(): o.get("ContentType") for o in ct.findall(f"{{{CT}}}Override")
    }
    for name in parts:
        if name == "/[Content_Types].xml":
            continue
        ext = name.rsplit(".", 1)[-1].lower()
        if name.lower() not in overrides and ext not in defaults:
            problems.append(f"{name}: no content type")
    for name in overrides:
        if name not in lower:
            problems.append(f"[Content_Types].xml: override for missing part {name}")

    # Relationships: targets exist; every r:* attribute resolves.
    rels: dict[str, dict[str, tuple[str, str, bool]]] = {}
    for name, body in parts.items():
        if not name.endswith(".rels"):
            continue
        d, f = posixpath.split(name)
        source = posixpath.join(posixpath.dirname(d), f[: -len(".rels")]) if f != ".rels" else "/"
        entries: dict[str, tuple[str, str, bool]] = {}
        for rel in etree.fromstring(body).findall(f"{{{PKG_RELS}}}Relationship"):
            rid = rel.get("Id")
            if rid in entries:
                problems.append(f"{name}: duplicate relationship id {rid}")
            external = rel.get("TargetMode") == "External"
            entries[rid] = (rel.get("Type"), rel.get("Target"), external)
            path = rel.get("Target").split("#", 1)[0]
            if not external and path:
                target = _resolve(source if source != "/" else "/x", path)
                if not exists(target):
                    problems.append(f"{name}: {rid} points at missing {target}")
        rels[source] = entries

    for name, body in parts.items():
        if not (name.endswith(".xml") or name.endswith(".rels")) or name.endswith(".rels"):
            continue
        if name == "/[Content_Types].xml":
            continue
        try:
            root = etree.fromstring(body)
        except etree.XMLSyntaxError as e:
            problems.append(f"{name}: not well-formed: {e}")
            continue
        known = rels.get(name, {})
        for el in root.iter():
            for key, value in el.attrib.items():
                if key.startswith(f"{{{R}}}") and value and value not in known:
                    problems.append(f"{name}: r:{key.split('}')[1]}={value} has no relationship")
        if name.startswith("/ppt/slides/slide"):
            problems += _check_slide(name, root)
        if name.startswith("/ppt/charts/chart"):
            problems += _check_chart(name, root)
        if name == "/ppt/presentation.xml":
            problems += _check_presentation(root)

    return problems


def _check_slide(name: str, root: etree._Element) -> list[str]:
    problems = []
    ids = Counter()
    for el in root.iter(f"{{{P}}}cNvPr"):
        # Shapes inside mc:Choice/mc:Fallback legitimately share ids.
        in_fallback = any(a.tag.endswith("}Fallback") for a in el.iterancestors())
        if not in_fallback:
            ids[el.get("id")] += 1
    dupes = [i for i, n in ids.items() if n > 1]
    if dupes:
        problems.append(f"{name}: duplicate shape ids {dupes}")
    existing = {el.get("id") for el in root.iter(f"{{{P}}}cNvPr")}
    for el in root.iter(f"{{{P}}}spTgt", f"{{{P}}}bldP", f"{{{P}}}bldGraphic", f"{{{P}}}bldDgm"):
        if el.get("spid") not in existing:
            problems.append(f"{name}: animation targets missing shape {el.get('spid')}")
    for el in root.iter(f"{{{A}}}stCxn", f"{{{A}}}endCxn"):
        if el.get("id") not in existing:
            problems.append(f"{name}: connector glued to missing shape {el.get('id')}")
    for sp in root.iter(f"{{{P}}}sp"):
        order = [etree.QName(c).localname for c in sp if etree.QName(c).namespace == P]
        ranks = [SP_ORDER.index(t) for t in order if t in SP_ORDER]
        if ranks != sorted(ranks):
            problems.append(f"{name}: p:sp children out of order: {order}")
    for tbl in root.iter(f"{{{A}}}tbl"):
        problems += _check_table(name, tbl)
    return problems


def _check_table(name: str, tbl: etree._Element) -> list[str]:
    problems = []
    cols = len(tbl.findall(f"{{{A}}}tblGrid/{{{A}}}gridCol"))
    rows = tbl.findall(f"{{{A}}}tr")
    for r, tr in enumerate(rows):
        cells = tr.findall(f"{{{A}}}tc")
        if len(cells) != cols:
            problems.append(f"{name}: table row {r} has {len(cells)} cells for {cols} columns")
        for c, tc in enumerate(cells):
            span = int(tc.get("gridSpan", "1"))
            row_span = int(tc.get("rowSpan", "1"))
            if c + span > cols:
                problems.append(f"{name}: cell ({r},{c}) spans past the last column")
            if r + row_span > len(rows):
                problems.append(f"{name}: cell ({r},{c}) spans past the last row")
            if tc.get("vMerge") == "1" and r == 0:
                problems.append(f"{name}: cell (0,{c}) continues a merge from nowhere")
    return problems


def _check_chart(name: str, root: etree._Element) -> list[str]:
    problems = []
    idx = []
    order = []
    for ser in root.iter():
        if not isinstance(ser.tag, str) or etree.QName(ser).localname != "ser":
            continue
        for child in ser:
            local = etree.QName(child).localname
            if local == "idx":
                idx.append(child.get("val"))
            elif local == "order":
                order.append(child.get("val"))
    for label, values in (("c:idx", idx), ("c:order", order)):
        dupes = [v for v, n in Counter(values).items() if n > 1]
        if dupes:
            problems.append(f"{name}: duplicate series {label} {dupes}")
    for plot in root.iter():
        if isinstance(plot.tag, str) and etree.QName(plot).namespace == C:
            local = etree.QName(plot).localname
            if local.endswith("Chart") and local != "chart" and plot.find(f"{{{C}}}ser") is None:
                problems.append(f"{name}: plot {local} has no series")
    for v in root.iter(f"{{{C}}}v"):
        if v.text and re.fullmatch(r"[+-]?(inf|nan)", v.text.strip(), re.IGNORECASE):
            problems.append(f"{name}: non-finite value {v.text!r}")
    for ax in ("axId", "crossAx"):
        for el in root.iter(f"{{{C}}}{ax}"):
            if el.get("val", "0").startswith("-"):
                problems.append(f"{name}: negative c:{ax}")
    return problems


def _check_presentation(root: etree._Element) -> list[str]:
    problems = []
    slide_ids = [e.get("id") for e in root.iter(f"{{{P}}}sldId")]
    if len(slide_ids) != len(set(slide_ids)):
        problems.append("presentation.xml: duplicate p:sldId ids")
    sections = [
        [e.get("id") for e in section.iter(f"{{{P14}}}sldId")]
        for section in root.iter(f"{{{P14}}}section")
    ]
    if sections:
        flat = [i for s in sections for i in s]
        if flat != slide_ids:
            problems.append(
                f"presentation.xml: sections {sections} don't match slide order {slide_ids}"
            )
    return problems
