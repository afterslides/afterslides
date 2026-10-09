from __future__ import annotations

import pytest

from afterslides import InvalidArgumentError, NotFoundError, Presentation, UnsupportedError
from conftest import as_python_pptx, reopen, zip_entries


def test_lists_shapes_with_kinds(prs: Presentation) -> None:
    kinds = {s.name: s.kind for slide in prs.slides for s in slide.shapes}
    assert kinds["Revenue Chart"] == "chart"
    assert kinds["Top Customers"] == "table"
    assert kinds["Card"] == "group"
    assert kinds["Grouped Label"] == "shape"
    assert kinds["Logo"] == "picture"


def test_shape_lookup(prs: Presentation) -> None:
    slide = prs.slides[5]
    label = slide.shape("Grouped Label")
    assert label.parent is not None and label.parent.name == "Card"
    assert "Logo" in slide
    assert prs.shape("Logo").alt_text == "logo:company"
    assert prs.find_shapes(alt_text="logo:company") == [prs.shape("Logo")]
    assert {s.name for s in prs.find_shapes(kind="chart")} == {
        "Revenue Chart",
        "Trend",
        "Share",
        "Scatter",
        "Bubbles",
    }
    with pytest.raises(NotFoundError):
        slide.shape("nope")
    with pytest.raises(NotFoundError):
        prs.shape("nope")


def test_ambiguous_deck_wide_lookup(prs: Presentation) -> None:
    prs.slides[1].duplicate()
    with pytest.raises(InvalidArgumentError):
        prs.shape("Revenue Chart")


def test_placeholders(prs: Presentation) -> None:
    title = prs.slides[0].shapes[0]
    assert title.is_placeholder
    assert title.placeholder_type == "ctrTitle"


def test_replace_text_across_runs_keeps_formatting(prs: Presentation) -> None:
    hits = prs.replace_text(
        {"{{title}}": "Q3", "{{customer}}": "Acme", "{{period}}": 2026, "{{name}}": "Ada"}
    )
    assert hits == 5  # four on slides, one in the speaker notes
    check = as_python_pptx(prs)
    subtitle = check.slides[0].placeholders[1].text_frame.paragraphs[0]
    assert subtitle.text == "Report for Acme — 2026"
    # "{{customer}}" started in the bold run, so "Acme" is bold.
    assert subtitle.runs[0].text == "Report for Acme"
    assert subtitle.runs[0].font.bold is True
    assert subtitle.runs[1].font.bold is False


def test_replace_text_leaves_untouched_slides_alone(prs: Presentation) -> None:
    before = zip_entries(prs)
    prs.replace_text({"{{name}}": "Ada"})
    after = zip_entries(prs)
    changed = {n for n in before if before[n] != after.get(n)}
    assert changed == {"ppt/slides/slide6.xml"}


def test_replace_text_scoped_to_slide_and_shape(prs: Presentation) -> None:
    assert prs.slides[1].replace_text({"{{title}}": "x"}, notes=False) == 0
    assert prs.slides[1].replace_text({"{{title}}": "x"}) == 1  # in the speaker notes
    assert prs.slides[1].notes == "Speaker notes for x"
    note = prs.slides[1].shape("Note")
    assert note.replace_text({"{{source}}": "ERP"}) == 1
    assert note.text == "Source: ERP"


def test_set_text(prs: Presentation) -> None:
    note = prs.slides[1].shape("Note")
    note.text = "line 1\nline 2\vbroken"
    deck = reopen(prs)
    assert deck.slides[1].shape("Note").text == "line 1\nline 2\vbroken"
    para = as_python_pptx(prs).slides[1].shapes[2].text_frame.paragraphs
    assert [p.text for p in para] == ["line 1", "line 2\vbroken"]


def test_set_text_on_non_text_shape_fails(prs: Presentation) -> None:
    with pytest.raises(UnsupportedError):
        prs.shape("Logo").text = "nope"


def test_delete_shape(prs: Presentation) -> None:
    prs.shape("Remove Me").delete()
    deck = reopen(prs)
    assert "Remove Me" not in deck.slides[3]
    with pytest.raises(NotFoundError):
        prs.slides[3].shape("Remove Me")


def test_delete_picture_drops_unused_media(prs: Presentation) -> None:
    assert any(n.startswith("ppt/media/") for n in zip_entries(prs))
    prs.shape("Logo").delete()
    assert not any(n.startswith("ppt/media/") for n in zip_entries(prs))


def test_delete_group_removes_members(prs: Presentation) -> None:
    prs.shape("Card").delete()
    assert [s.name for s in reopen(prs).slides[5].shapes] == ["Logo"]


def test_stale_handle_raises(prs: Presentation) -> None:
    shape = prs.shape("Remove Me")
    shape.delete()
    with pytest.raises(NotFoundError):
        shape.text = "gone"


def test_replacement_values_with_line_breaks(prs: Presentation) -> None:
    note = prs.slides[1].shape("Note")
    note.replace_text({"{{source}}": "ERP\nFinance\vteam"})
    assert reopen(prs).slides[1].shape("Note").text == "Source: ERP\nFinance\vteam"
    paragraphs = as_python_pptx(prs).slides[1].shapes[2].text_frame.paragraphs
    assert [p.text for p in paragraphs] == ["Source: ERP", "Finance\vteam"]


def test_replace_text_reaches_chart_titles(prs: Presentation) -> None:
    prs.shape("Revenue Chart").chart.title = "Revenue {{year}}"
    assert prs.replace_text({"{{year}}": 2026}) == 1
    assert reopen(prs).shape("Revenue Chart").chart.title == "Revenue 2026"


def test_slide_notes(prs: Presentation) -> None:
    slide = prs.slides[1]
    assert slide.notes == "Speaker notes for {{title}}"
    slide.notes = "First point\nSecond point"
    prs.slides[0].notes = "New notes on a slide that had none"
    check = as_python_pptx(prs)
    assert check.slides[1].notes_slide.notes_text_frame.text == "First point\nSecond point"
    assert check.slides[0].notes_slide.notes_text_frame.text == (
        "New notes on a slide that had none"
    )
    assert prs.slides[2].notes == ""
