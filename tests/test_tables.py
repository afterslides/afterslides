from __future__ import annotations

import pytest

from afterslides import InvalidArgumentError, Presentation, Table, UnsupportedError
from conftest import as_python_pptx, reopen


def table_of(prs: Presentation) -> Table:
    return prs.shape("Top Customers").table


def test_read_table(prs: Presentation) -> None:
    table = table_of(prs)
    assert table.size == (3, 3)
    assert table[0, 0] == "Customer"
    assert table.values[1] == ["Acme", "1.2M", "30%"]


def test_set_cell(prs: Presentation) -> None:
    table = table_of(prs)
    table[1, 1] = 3.5
    table[2, 2] = None
    assert table_of(reopen(prs)).values[1:] == [["Acme", "3.5", "30%"], ["Globex", "0.8M", ""]]


def test_fill_grows_table_copying_row_formatting(prs: Presentation) -> None:
    rows = [(f"Customer {i}", f"{i}.0M", f"{i}%") for i in range(5)]
    table_of(prs).fill(rows)
    deck = reopen(prs)
    assert table_of(deck).values == [["Customer", "Revenue", "Share"], *map(list, rows)]
    # python-pptx agrees, and the new rows kept the 14pt font of the template row.
    check = as_python_pptx(prs).slides[2].shapes[1].table
    assert len(check.rows) == 6
    assert check.cell(5, 0).text_frame.paragraphs[0].runs[0].font.size.pt == 14


def test_fill_shrinks_table(prs: Presentation) -> None:
    table_of(prs).fill([["Only", "1", "2"]])
    assert table_of(reopen(prs)).size == (2, 3)


def test_fill_with_no_data_keeps_header(prs: Presentation) -> None:
    table_of(prs).fill([])
    assert table_of(reopen(prs)).values == [["Customer", "Revenue", "Share"]]


def test_fill_without_resize(prs: Presentation) -> None:
    table = table_of(prs)
    table.fill([["A"]], resize=False)
    assert table.values[1] == ["A", "1.2M", "30%"]
    assert table.rows == 3
    with pytest.raises(InvalidArgumentError):
        table.fill([["a"], ["b"], ["c"]], resize=False)


def test_too_many_columns(prs: Presentation) -> None:
    with pytest.raises(InvalidArgumentError):
        table_of(prs).fill([["a", "b", "c", "d"]])


def test_frame_height_follows_rows(prs: Presentation) -> None:
    before = prs.shape("Top Customers").frame
    table_of(prs).fill([["x"]] * 6)
    after = reopen(prs).shape("Top Customers").frame
    assert before is not None and after is not None
    assert after[3] > before[3]


def test_row_and_column_operations(prs: Presentation) -> None:
    table = table_of(prs)
    table.insert_row(1, copy_of=1)
    assert table.values[1] == ["", "", ""]
    table.delete_row(-1)
    table.delete_column(0)
    assert table_of(reopen(prs)).values == [["Revenue", "Share"], ["", ""], ["1.2M", "30%"]]
    check = as_python_pptx(prs).slides[2].shapes[1].table
    assert len(check.columns) == 2


def test_table_on_wrong_shape(prs: Presentation) -> None:
    with pytest.raises(UnsupportedError):
        _ = prs.shape("Logo").table
