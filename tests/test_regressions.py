"""Regression tests for ways real-world templates used to come out broken.

Each test patches the fixture so it contains a construct found in decks
saved by PowerPoint, then checks the result with the invariant checker
(via zip_entries/reopen) and, where useful, with python-pptx.
"""

from __future__ import annotations

import re

import pytest

from afterslides import InvalidArgumentError, Presentation, UnsupportedError
from conftest import as_python_pptx, patched, reopen, replace_once, zip_entries

SLIDE_RELS = "ppt/slides/_rels/slide{}.xml.rels"


def test_percent_encoded_media_target_survives_cleanup() -> None:
    data = patched(
        {
            SLIDE_RELS.format(6): replace_once(
                'Target="../media/image1.png"', 'Target="../media/image%201.png"'
            )
        },
        renames={"ppt/media/image1.png": "ppt/media/image%201.png"},
    )
    prs = Presentation(data)
    prs.shape("Trend").delete()  # releases a chart, so unused parts get cleaned up
    assert "ppt/media/image%201.png" in zip_entries(prs)


def test_media_target_with_different_case_survives_cleanup() -> None:
    data = patched(
        {
            SLIDE_RELS.format(6): replace_once(
                'Target="../media/image1.png"', 'Target="../media/IMAGE1.PNG"'
            )
        }
    )
    prs = Presentation(data)
    prs.shape("Trend").delete()
    assert "ppt/media/image1.png" in zip_entries(prs)


def test_duplicate_shape_ids_are_made_unique() -> None:
    # PowerPoint tolerates (and silently fixes) duplicate ids; we must not
    # delete the wrong shape because of them.
    data = patched(
        {
            "ppt/slides/slide4.xml": replace_once(
                'id="4" name="Remove Me"', 'id="2" name="Remove Me"'
            )
        }
    )
    prs = Presentation(data)
    shapes = prs.slides[3].shapes
    assert len({s.id for s in shapes}) == len(shapes)
    prs.shape("Remove Me").delete()
    deck = reopen(prs)
    assert [s.name for s in deck.slides[3].shapes] == ["Trend", "Share"]


MODERN_COMMENTS = (
    '<p:extLst><p:ext uri="{6950BFC3-D8DA-4A85-94F7-54DA5524770B}">'
    '<p188:commentRel xmlns:p188="http://schemas.microsoft.com/office/powerpoint/2018/8/main"'
    ' r:id="rId9"/></p:ext></p:extLst></p:sld>'
)


def test_duplicating_slide_with_modern_comments() -> None:
    data = patched(
        {
            "ppt/slides/slide2.xml": replace_once("</p:sld>", MODERN_COMMENTS),
            SLIDE_RELS.format(2): replace_once(
                "</Relationships>",
                '<Relationship Id="rId9" Type="http://schemas.microsoft.com/office/2018/10/'
                'relationships/comments" Target="../comments/modernComment_1.xml"/>'
                "</Relationships>",
            ),
            "ppt/comments/modernComment_1.xml": (
                '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                '<p188:cmLst xmlns:p188="http://schemas.microsoft.com/office/powerpoint/2018/8/main"/>'
            ),
            "[Content_Types].xml": replace_once(
                "</Types>",
                '<Override PartName="/ppt/comments/modernComment_1.xml"'
                ' ContentType="application/vnd.ms-powerpoint.comments+xml"/></Types>',
            ),
        }
    )
    prs = Presentation(data)
    prs.slides[1].duplicate()
    entries = zip_entries(prs)  # fails on a dangling r:id
    assert "ppt/comments/modernComment_1.xml" in entries


FILTERED_SERIES = (
    '<c:extLst><c:ext uri="{02D57815-91ED-43cb-92C2-25804820EDAC}"'
    ' xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart">'
    '<c15:filteredBarSeries><c15:ser><c:idx val="2"/><c:order val="2"/>'
    '<c:tx><c:v>hidden</c:v></c:tx><c:val><c:numLit><c:ptCount val="0"/></c:numLit></c:val>'
    "</c15:ser></c15:filteredBarSeries></c:ext></c:extLst></c:barChart>"
)


def test_new_series_avoid_ids_of_filtered_series() -> None:
    data = patched({"ppt/charts/chart1.xml": replace_once("</c:barChart>", FILTERED_SERIES)})
    prs = Presentation(data)
    prs.shape("Revenue Chart").chart.replace_data(["a"], {"x": [1], "y": [2], "z": [3]})
    zip_entries(prs)  # fails on duplicate c:idx


BARE_SHAPE = (
    '<p:sp><p:nvSpPr><p:cNvPr id="10" name="Bare"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
    '<p:spPr/><p:extLst><p:ext uri="{C183D7F6-B498-43B3-948B-1728B52AA6E4}">'
    '<a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main"'
    ' id="{00000000-0000-0000-0000-000000000001}"/></p:ext></p:extLst></p:sp></p:spTree>'
)


def test_text_body_goes_before_ext_list() -> None:
    data = patched({"ppt/slides/slide4.xml": replace_once("</p:spTree>", BARE_SHAPE)})
    prs = Presentation(data)
    prs.shape("Bare").text = "now with text"
    assert reopen(prs).shape("Bare").text == "now with text"


@pytest.mark.parametrize("bad", [float("inf"), float("-inf")])
def test_non_finite_chart_values_are_rejected(prs: Presentation, bad: float) -> None:
    chart = prs.shape("Revenue Chart").chart
    with pytest.raises(InvalidArgumentError):
        chart.replace_data(["a"], {"s": [bad]})
    with pytest.raises(InvalidArgumentError):
        prs.shape("Trend").chart.replace_data([1.0, bad], {"s": [1, 2]})


def merged_table(text: str) -> str:
    """Merges cells (1,0) and (2,0) of the fixture table vertically."""
    cells = [m.start() for m in re.finditer(r"<a:tc>", text)]
    assert len(cells) == 9
    row2, row1 = cells[6], cells[3]
    text = text[:row2] + '<a:tc vMerge="1">' + text[row2 + len("<a:tc>") :]
    return text[:row1] + '<a:tc rowSpan="2">' + text[row1 + len("<a:tc>") :]


@pytest.fixture
def merged() -> Presentation:
    return Presentation(patched({"ppt/slides/slide3.xml": merged_table}))


def test_shrinking_table_shortens_vertical_merge(merged: Presentation) -> None:
    merged.shape("Top Customers").table.fill([["only", "1", "2"]])
    zip_entries(merged)  # fails on a span past the last row


def test_deleting_merge_head_row_promotes_next_cell(merged: Presentation) -> None:
    table = merged.shape("Top Customers").table
    table.delete_row(1)
    deck = reopen(merged)
    assert deck.shape("Top Customers").table.values[1] == ["Acme", "0.8M", "20%"]


def test_growing_table_with_vertical_merge(merged: Presentation) -> None:
    merged.shape("Top Customers").table.fill([[f"c{i}", i, i] for i in range(5)])
    zip_entries(merged)
    check = as_python_pptx(merged).slides[2].shapes[1].table
    assert len(check.rows) == 6


def test_inserting_row_inside_vertical_merge_extends_it(merged: Presentation) -> None:
    merged.shape("Top Customers").table.insert_row(2, copy_of=1)
    entries = zip_entries(merged)
    xml = entries["ppt/slides/slide3.xml"].decode()
    assert 'rowSpan="3"' in xml


def combo_chart(text: str) -> str:
    """Adds a line plot with one series to the clustered column chart."""
    ser = re.search(r"<c:ser>.*?</c:ser>", text, re.S)
    assert ser is not None
    line_ser = re.sub(r"<c:invertIfNegative[^>]*/>", "", ser.group(0))
    line_ser = re.sub(r'<c:idx val="\d+"/>', '<c:idx val="5"/>', line_ser)
    line_ser = re.sub(r'<c:order val="\d+"/>', '<c:order val="5"/>', line_ser)
    axes = re.findall(r'<c:axId val="\d+"/>', text)[:2]
    line = (
        '<c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>'
        + line_ser
        + '<c:marker val="1"/>'
        + "".join(axes)
        + "</c:lineChart>"
    )
    return text.replace("</c:barChart>", "</c:barChart>" + line, 1)


def test_combo_chart_keeps_series_in_their_plots() -> None:
    prs = Presentation(patched({"ppt/charts/chart1.xml": combo_chart}))
    chart = prs.shape("Revenue Chart").chart
    assert chart.types == ["barChart", "lineChart"]
    chart.replace_data(["a", "b"], {"bars 1": [1, 2], "bars 2": [3, 4], "line": [5, 6]})
    deck = reopen(prs)
    assert [s.name for s in deck.shape("Revenue Chart").chart.series] == [
        "bars 1",
        "bars 2",
        "line",
    ]
    plots = as_python_pptx(prs).slides[1].shapes[1].chart.plots
    assert [len(p.series) for p in plots] == [2, 1]


@pytest.mark.parametrize("count", [1, 2, 4])
def test_combo_chart_refuses_to_guess_series_placement(count: int) -> None:
    prs = Presentation(patched({"ppt/charts/chart1.xml": combo_chart}))
    chart = prs.shape("Revenue Chart").chart
    with pytest.raises(UnsupportedError):
        chart.replace_data(["a"], {f"s{i}": [i] for i in range(count)})


CREATION_ID = (
    '<p:extLst><p:ext uri="{BB962C8B-B14F-4D97-AF65-F5344CB8AC3E}">'
    '<p14:creationId xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"'
    ' val="1234"/></p:ext></p:extLst></p:sld>'
)


def test_duplicated_slide_gets_its_own_creation_id() -> None:
    prs = Presentation(patched({"ppt/slides/slide2.xml": replace_once("</p:sld>", CREATION_ID)}))
    copy = prs.slides[1].duplicate()
    entries = zip_entries(prs)
    ids = [
        re.search(r'creationId[^>]*val="(\d+)"', entries[s.part_name.lstrip("/")].decode())
        for s in (prs.slides[1], copy)
    ]
    assert ids[0] is not None and ids[1] is not None
    assert ids[0].group(1) == "1234"
    assert ids[1].group(1) != "1234"
