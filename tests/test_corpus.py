"""Runs every operation against real-world decks.

The corpus (scripts/fetch_corpus.sh) holds decks saved by PowerPoint,
LibreOffice and others, taken from the test suites of other OOXML projects.
For each deck we apply each kind of edit and check that the result has no
structural problems the input didn't already have. With
AFTERSLIDES_DUMP_DIR set, results are also written out for the Open XML SDK
validator.

Expected refusals (UnsupportedError for combo charts and the like) are fine;
any other exception is a bug.
"""

from __future__ import annotations

import io
import os
import zipfile
from collections.abc import Callable
from pathlib import Path

import pytest

from afterslides import (
    PackageError,
    Presentation,
    Series,
    UnsupportedError,
    XySeries,
)
from conftest import dump
from invariants import check_deck, problems

ROOT = Path(__file__).resolve().parent.parent
CORPUS = Path(os.environ.get("AFTERSLIDES_CORPUS", ROOT / ".corpus"))
DECKS = sorted(CORPUS.rglob("*.pptx")) if CORPUS.is_dir() else []

pytestmark = [
    pytest.mark.corpus,
    pytest.mark.skipif(not DECKS, reason="no corpus; run scripts/fetch_corpus.sh"),
]


def load(path: Path) -> tuple[bytes, Presentation] | None:
    data = path.read_bytes()
    try:
        return data, Presentation(data)
    except PackageError:
        return None  # broken on purpose (fuzzer output, truncated files)


def finish(original: bytes, prs: Presentation, label: str, source: Path) -> None:
    out = prs.to_bytes()
    check_deck(out, baseline=original)
    dump(out, label, source)
    # And it must open again.
    Presentation(out)


def apply(path: Path, op: Callable[[Presentation], None], label: str) -> None:
    loaded = load(path)
    if loaded is None:
        pytest.skip("not a readable package")
    data, prs = loaded
    op(prs)
    finish(data, prs, f"{path.stem}-{label}", path)


def ids(paths: list[Path]) -> list[str]:
    return [f"{p.parent.name}/{p.name}" for p in paths]


@pytest.mark.parametrize("path", DECKS, ids=ids(DECKS))
def test_untouched_round_trip(path: Path) -> None:
    loaded = load(path)
    if loaded is None:
        pytest.skip("not a readable package")
    data, prs = loaded
    out = prs.to_bytes()
    with zipfile.ZipFile(io.BytesIO(data)) as a, zipfile.ZipFile(io.BytesIO(out)) as b:
        changed = [n for n in a.namelist() if not n.endswith("/") and a.read(n) != b.read(n)]
    # Only slides with duplicate shape ids may be rewritten (on open).
    dupes = {p.split(":")[0] for p in problems(data) if "duplicate shape ids" in p}
    assert [n for n in changed if f"/{n}" not in dupes] == []


@pytest.mark.parametrize("path", DECKS, ids=ids(DECKS))
def test_read_everything(path: Path) -> None:
    loaded = load(path)
    if loaded is None:
        pytest.skip("not a readable package")
    _, prs = loaded
    for slide in prs.slides:
        for shape in slide.shapes:
            _ = (shape.name, shape.kind, shape.text, shape.frame, shape.alt_text)
            if shape.is_table:
                _ = shape.table.values
            if shape.is_chart:
                chart = shape.chart
                _ = (chart.types, chart.title)
                _ = chart.xy_series if chart.is_xy else (chart.categories, chart.series)


def replace_text(prs: Presentation) -> None:
    prs.replace_text({"e": "€", "a": "{{a}}", "The": ""})


def set_all_text(prs: Presentation) -> None:
    for slide in prs.slides:
        for shape in slide.shapes:
            if shape.kind == "shape":
                shape.text = f"{shape.name}\nline two\vbroken"


def refill_charts(prs: Presentation) -> None:
    """Writes each chart's own data back, then a bigger variant."""
    for slide in prs.slides:
        for shape in slide.shapes:
            if not shape.is_chart:
                continue
            chart = shape.chart
            try:
                if chart.is_xy:
                    series = chart.xy_series
                    chart.replace_xy_data(series)
                    extra = XySeries(
                        "extra", [1, 2], [3, 4], [5, 6] if series and series[0].sizes else None
                    )
                    chart.replace_xy_data([*series, extra])
                else:
                    cats = chart.categories
                    series = chart.series
                    chart.replace_data(cats, series)
                    n = len(cats) + 1
                    bigger = [Series(s.name, [*s.values, 1.0]) for s in series]
                    chart.replace_data(
                        [*map(str, cats), "new"], [*bigger, Series("extra", [2.0] * n)]
                    )
            except UnsupportedError:
                pass


def fill_tables(prs: Presentation) -> None:
    for slide in prs.slides:
        for shape in slide.shapes:
            if shape.is_table:
                table = shape.table
                rows, cols = table.size
                table.fill([[f"{r}/{c}" for c in range(cols)] for r in range(rows + 2)])
                table.fill([["x"] * cols])


def delete_every_other_shape(prs: Presentation) -> None:
    for slide in prs.slides:
        top_level = [s for s in slide.shapes if s.parent is None]
        for shape in top_level[::2]:
            shape.delete()


def duplicate_then_delete_originals(prs: Presentation) -> None:
    originals = list(prs.slides)
    for slide in originals:
        slide.duplicate()
    for slide in originals:
        slide.delete()


def delete_every_other_slide(prs: Presentation) -> None:
    for i, slide in enumerate(list(prs.slides)):
        if i % 2:
            slide.delete()
    if len(prs.slides) > 1:
        prs.slides[-1].move_to(0)


OPERATIONS = {
    "replace_text": replace_text,
    "set_text": set_all_text,
    "charts": refill_charts,
    "tables": fill_tables,
    "delete_shapes": delete_every_other_shape,
    "duplicate_slides": duplicate_then_delete_originals,
    "delete_slides": delete_every_other_slide,
}


@pytest.mark.parametrize("op", list(OPERATIONS))
@pytest.mark.parametrize("path", DECKS, ids=ids(DECKS))
def test_operation(path: Path, op: str) -> None:
    apply(path, OPERATIONS[op], op)
