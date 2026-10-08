from __future__ import annotations

import re

import pytest

from afterslides import InvalidArgumentError, NotFoundError, Presentation
from conftest import as_python_pptx, reopen, zip_entries


def section_ids(prs: Presentation) -> list[list[int]]:
    xml = zip_entries(prs)["ppt/presentation.xml"].decode()
    sections = re.findall(r"<p14:section .*?</p14:section>", xml)
    return [[int(i) for i in re.findall(r'<p14:sldId id="(\d+)"', s)] for s in sections]


def slide_ids(prs: Presentation) -> list[int]:
    return [s.id for s in prs.slides]


def test_delete_slide(prs: Presentation) -> None:
    doomed = prs.slides[3]
    doomed.delete()
    deck = reopen(prs)
    assert len(deck.slides) == 6
    assert all(doomed.id not in ids for ids in section_ids(prs))
    assert len(as_python_pptx(prs).slides) == 6
    with pytest.raises(NotFoundError):
        _ = doomed.index
    entries = zip_entries(prs)
    assert b"<Slides>6</Slides>" in entries["docProps/app.xml"]


def test_delete_while_iterating(prs: Presentation) -> None:
    for position, slide in enumerate(prs.slides):
        if position % 2:
            slide.delete()
    assert len(reopen(prs).slides) == 4


def test_delete_slide_with_notes(prs: Presentation) -> None:
    prs.slides[1].delete()
    assert not any("notesSlide" in n for n in zip_entries(prs))


def test_duplicate_slide(prs: Presentation) -> None:
    original = prs.slides[1]
    copy = original.duplicate()
    assert copy.index == 2
    copy.shape("Revenue Chart").chart.replace_data(["x"], {"only": [1]})
    copy.replace_text({"{{source}}": "copy"})
    deck = reopen(prs)
    assert len(deck.slides) == 8
    assert len(deck.slides[1].shape("Revenue Chart").chart.series) == 2
    assert len(deck.slides[2].shape("Revenue Chart").chart.series) == 1
    assert deck.slides[1].shape("Note").text == "Source: {{source}}"
    assert deck.slides[2].shape("Note").text == "Source: copy"
    # The copy joins the original's section, right after it.
    assert section_ids(prs)[0] == [*slide_ids(prs)[:4]]
    check = as_python_pptx(prs)
    assert check.slides[2].notes_slide.notes_text_frame.text == "Speaker notes for {{title}}"


def test_duplicate_to_position(prs: Presentation) -> None:
    copy = prs.slides[0].duplicate(position=7)
    assert copy.index == 7
    assert section_ids(prs)[-1][-1] == copy.id
    with pytest.raises(InvalidArgumentError):
        prs.slides[0].duplicate(position=99)


def test_one_slide_per_item(prs: Presentation) -> None:
    template = prs.slides[1]
    for i, region in enumerate(["North", "South", "West"]):
        slide = template.duplicate(position=template.index + 1 + i)
        slide.replace_text({"{{source}}": region})
        slide.shape("Revenue Chart").chart.replace_data(["Q1"], {region: [i]})
    template.delete()
    deck = reopen(prs)
    notes = [deck.slides[i].shape("Note").text for i in range(1, 4)]
    assert notes == ["Source: North", "Source: South", "Source: West"]
    assert len(as_python_pptx(prs).slides) == 9


def test_move_slide(prs: Presentation) -> None:
    last = prs.slides[-1]
    last.move_to(0)
    assert last.index == 0
    assert slide_ids(reopen(prs))[0] == last.id
    # Sections stay contiguous and in slide order.
    flat = [i for ids in section_ids(prs) for i in ids]
    assert flat == slide_ids(prs)
    with pytest.raises(InvalidArgumentError):
        last.move_to(7)
