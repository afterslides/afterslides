from __future__ import annotations

import datetime as _dt
import math
import os
from collections.abc import Iterable, Iterator, Mapping, Sequence
from dataclasses import dataclass, field
from typing import IO, Any, Literal, Union, overload

from afterslides import _native
from afterslides.errors import InvalidArgumentError, NotFoundError, UnsupportedError

Source = Union[str, "os.PathLike[str]", bytes, bytearray, memoryview, IO[bytes]]
Target = Union[str, "os.PathLike[str]", IO[bytes]]
Number = Union[int, float, None]

_EXCEL_EPOCH = _dt.datetime(1899, 12, 30)


def _to_float(value: Any) -> float | None:
    """Converts a chart value to float; ``None`` and NaN become gaps."""
    if value is None:
        return None
    try:
        number = float(value)
    except (TypeError, ValueError):
        raise InvalidArgumentError(f"chart values must be numbers or None, got {value!r}") from None
    return None if math.isnan(number) else number


def _excel_serial(value: _dt.date) -> float:
    if not isinstance(value, _dt.datetime):
        value = _dt.datetime(value.year, value.month, value.day)
    delta = value.replace(tzinfo=None) - _EXCEL_EPOCH
    return delta.days + delta.seconds / 86400 + delta.microseconds / 86_400_000_000


def _categories(values: Iterable[Any]) -> tuple[list[str] | list[float] | list[list[str]], bool]:
    """Categories in the form the native layer takes, plus whether they are dates."""
    values = list(values)
    if values and all(isinstance(v, (_dt.date, _dt.datetime)) for v in values):
        return [_excel_serial(v) for v in values], True
    if values and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in values):
        return [float(v) for v in values], False
    if values and all(isinstance(v, (tuple, list)) for v in values):
        return [[_text(level) for level in v] for v in values], False
    return [_text(v) for v in values], False


def _text(value: Any) -> str:
    return "" if value is None else str(value)


def _replacements(mapping: Mapping[str, Any]) -> list[tuple[str, str]]:
    return [(str(k), _text(v)) for k, v in mapping.items()]


@dataclass
class Series:
    """A named series of a category chart; one value per category.

    ``plot`` is only needed for combo charts: the index (into
    :attr:`Chart.types`) of the plot the series belongs to, for example 0 for
    the columns and 1 for the line. ``number_format`` overrides the
    template's number format for the values (e.g. ``"0.0%"``).
    """

    name: str
    values: list[float | None] = field(default_factory=list)
    plot: int | None = None
    number_format: str | None = field(default=None, compare=False)


@dataclass
class XySeries:
    """A series of a scatter or bubble chart."""

    name: str
    x: list[float | None] = field(default_factory=list)
    y: list[float | None] = field(default_factory=list)
    sizes: list[float | None] | None = None


class Presentation:
    """A PowerPoint deck opened from a file, bytes or a binary stream.

    >>> prs = Presentation("template.pptx")
    >>> prs.replace_text({"{{title}}": "Q3 report"})
    >>> prs.save("report.pptx")
    """

    __slots__ = ("_native",)

    def __init__(self, source: Source) -> None:
        if isinstance(source, (bytes, bytearray, memoryview)):
            self._native = _native.Presentation.from_bytes(bytes(source))
        elif isinstance(source, (str, os.PathLike)):
            self._native = _native.Presentation.open(os.fspath(source))
        elif hasattr(source, "read"):
            self._native = _native.Presentation.from_bytes(source.read())
        else:
            raise TypeError(f"cannot open a presentation from {type(source).__name__}")

    @classmethod
    def open(cls, source: Source) -> Presentation:
        """Same as ``Presentation(source)``."""
        return cls(source)

    def save(self, target: Target) -> None:
        """Writes the deck to a path or a writable binary stream.

        Saving to a path goes through a temporary file, so the template can be
        overwritten safely.
        """
        if isinstance(target, (str, os.PathLike)):
            self._native.save(os.fspath(target))
        else:
            target.write(self._native.to_bytes())

    def to_bytes(self) -> bytes:
        return bytes(self._native.to_bytes())

    @property
    def slides(self) -> Slides:
        return Slides(self)

    @property
    def slide_size(self) -> tuple[int, int]:
        """Width and height of the slides in EMU (914400 per inch)."""
        return self._native.slide_size()

    def replace_text(self, replacements: Mapping[str, Any], *, notes: bool = True) -> int:
        """Replaces placeholder text on all slides; returns the number of hits.

        Covers shapes, tables, chart titles and, unless ``notes=False``,
        speaker notes. Values are converted with ``str()``; ``\\n`` in a value
        starts a new paragraph, ``\\v`` a new line. Placeholders are found even
        when PowerPoint split them across differently formatted runs; the
        replacement takes the formatting of the run where the placeholder
        starts.
        """
        return self._native.replace_text(_replacements(replacements), notes=notes)

    def find_shapes(
        self,
        name: str | None = None,
        *,
        alt_text: str | None = None,
        kind: str | None = None,
    ) -> list[Shape]:
        """All shapes on all slides that match every given criterion."""
        return [
            s
            for slide in self.slides
            for s in slide.find_shapes(name, alt_text=alt_text, kind=kind)
        ]

    def shape(self, name: str) -> Shape:
        """The one shape in the deck with this name.

        Raises :class:`NotFoundError` if there is none and
        :class:`InvalidArgumentError` if the name is used more than once; use
        :meth:`Slide.shape` or :meth:`find_shapes` then.
        """
        found = self.find_shapes(name)
        if not found:
            raise NotFoundError(f"no shape named {name!r}")
        if len(found) > 1:
            where = ", ".join(str(s.slide.index) for s in found)
            raise InvalidArgumentError(f"{len(found)} shapes are named {name!r} (slides {where})")
        return found[0]

    def __repr__(self) -> str:
        return f"<Presentation with {len(self.slides)} slides>"


class Slides(Sequence["Slide"]):
    """The slides of a presentation, in order. A live view: it reflects
    deletions and insertions made after it was obtained."""

    __slots__ = ("_prs",)

    def __init__(self, prs: Presentation) -> None:
        self._prs = prs

    def _ids(self) -> list[int]:
        return self._prs._native.slides()

    def __len__(self) -> int:
        return len(self._ids())

    @overload
    def __getitem__(self, index: int) -> Slide: ...
    @overload
    def __getitem__(self, index: slice) -> list[Slide]: ...
    def __getitem__(self, index: int | slice) -> Slide | list[Slide]:
        ids = self._ids()
        if isinstance(index, slice):
            return [Slide(self._prs, i) for i in ids[index]]
        try:
            return Slide(self._prs, ids[index])
        except IndexError:
            raise IndexError(f"slide index {index} out of range ({len(ids)} slides)") from None

    def __iter__(self) -> Iterator[Slide]:
        # Snapshot, so deleting slides while iterating is safe.
        return iter([Slide(self._prs, i) for i in self._ids()])


class Slide:
    """One slide. Stays valid while the slide exists, even if other slides
    are added, removed or moved."""

    __slots__ = ("_prs", "id")

    def __init__(self, prs: Presentation, slide_id: int) -> None:
        self._prs = prs
        self.id = slide_id

    @property
    def _n(self) -> _native.Presentation:
        return self._prs._native

    @property
    def presentation(self) -> Presentation:
        return self._prs

    @property
    def index(self) -> int:
        """Current 0-based position in the deck."""
        return self._n.slide_index(self.id)

    @property
    def part_name(self) -> str:
        return self._n.slide_part(self.id)

    @property
    def shapes(self) -> list[Shape]:
        """All shapes, in z-order; group members follow their group."""
        return [Shape(self, info) for info in self._n.shapes(self.id)]

    def find_shapes(
        self,
        name: str | None = None,
        *,
        alt_text: str | None = None,
        kind: str | None = None,
    ) -> list[Shape]:
        return [
            s
            for s in self.shapes
            if (name is None or s.name == name)
            and (alt_text is None or s.alt_text == alt_text)
            and (kind is None or s.kind == kind)
        ]

    def shape(self, name: str) -> Shape:
        """The shape with this name (as shown in PowerPoint's selection pane).

        Raises :class:`NotFoundError` if there is none and
        :class:`InvalidArgumentError` if several shapes share the name; use
        :meth:`find_shapes` then.
        """
        found = [s for s in self.shapes if s.name == name]
        if not found:
            raise NotFoundError(f"no shape named {name!r} on slide {self.index}")
        if len(found) > 1:
            raise InvalidArgumentError(
                f"{len(found)} shapes are named {name!r} on slide {self.index}"
            )
        return found[0]

    def __contains__(self, name: object) -> bool:
        return any(s.name == name for s in self.shapes)

    def replace_text(self, replacements: Mapping[str, Any], *, notes: bool = True) -> int:
        """Like :meth:`Presentation.replace_text`, for this slide only."""
        return self._n.replace_text(_replacements(replacements), self.id, notes=notes)

    @property
    def hidden(self) -> bool:
        """Hidden slides stay in the file but are skipped in slide shows."""
        return self._n.slide_hidden(self.id)

    @hidden.setter
    def hidden(self, value: bool) -> None:
        self._n.set_slide_hidden(self.id, bool(value))

    @property
    def placeholders(self) -> list[Shape]:
        """Shapes that fill a placeholder of the slide layout."""
        return [s for s in self.shapes if s.is_placeholder]

    def placeholder(self, type: str | None = None, *, idx: int | None = None) -> Shape:
        """The placeholder with the given type (``title``, ``body``,
        ``ctrTitle``, ``subTitle``, ``pic``, ``chart``, ...) and/or index.

        Placeholders without an explicit type are body placeholders, as in
        PowerPoint.
        """
        if type is None and idx is None:
            raise InvalidArgumentError("give a placeholder type, an index or both")
        found = [
            s
            for s in self.placeholders
            if (type is None or s.placeholder_type == type)
            and (idx is None or s.placeholder_idx == idx)
        ]
        if not found:
            raise NotFoundError(f"no placeholder type={type!r} idx={idx!r} on slide {self.index}")
        return found[0]

    @property
    def notes(self) -> str:
        """Speaker notes; empty if the slide has none."""
        return self._n.slide_notes(self.id)

    @notes.setter
    def notes(self, value: Any) -> None:
        self._n.set_slide_notes(self.id, _text(value))

    def delete(self) -> None:
        """Removes the slide, its notes and charts. Hyperlinks to it on other
        slides are removed as well."""
        self._n.delete_slide(self.id)

    def duplicate(self, position: int | None = None) -> Slide:
        """Inserts a copy (right after this slide unless `position` is given).

        Charts and notes are copied, so the copy can be filled with different
        data. Typical use: one slide per item of a list.
        """
        return Slide(self._prs, self._n.duplicate_slide(self.id, position))

    def move_to(self, position: int) -> None:
        self._n.move_slide(self.id, position)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Slide) and other._prs is self._prs and other.id == self.id

    def __hash__(self) -> int:
        return hash((id(self._prs), self.id))

    def __repr__(self) -> str:
        try:
            return f"<Slide {self.index} id={self.id}>"
        except NotFoundError:
            return f"<Slide id={self.id} (deleted)>"


class Shape:
    """A shape on a slide: text box, placeholder, picture, table, chart, group...

    A handle: identifying properties (name, kind, placeholder) are read once;
    properties edits can change (frame, text, hidden) are read live.
    """

    __slots__ = ("_info", "slide")

    def __init__(self, slide: Slide, info: _native.ShapeInfo) -> None:
        self.slide = slide
        self._info = info

    @property
    def _n(self) -> _native.Presentation:
        return self.slide._prs._native

    @property
    def id(self) -> int:
        return int(self._info.id)

    @property
    def name(self) -> str:
        return str(self._info.name)

    @property
    def alt_text(self) -> str:
        """Alt text (description); a good place for template markers."""
        return str(self._info.alt_text)

    @property
    def title(self) -> str:
        return str(self._info.title)

    @property
    def kind(self) -> str:
        """One of ``shape``, ``picture``, ``table``, ``chart``, ``chartex``,
        ``smartart``, ``ole_object``, ``group``, ``connector``, ``other``."""
        return str(self._info.kind)

    @property
    def is_placeholder(self) -> bool:
        return self._info.placeholder_type is not None

    @property
    def placeholder_type(self) -> str | None:
        """Placeholder type (``title``, ``body``, ...), or None if this shape
        is not a placeholder."""
        return self._info.placeholder_type

    @property
    def placeholder_idx(self) -> int | None:
        return self._info.placeholder_idx

    def _live(self) -> _native.ShapeInfo:
        return self._n.shape_info(self.slide.id, self.id)

    @property
    def hidden(self) -> bool:
        return bool(self._live().hidden)

    @property
    def has_text_frame(self) -> bool:
        return bool(self._live().has_text)

    @property
    def is_chart(self) -> bool:
        return self.kind == "chart"

    @property
    def is_table(self) -> bool:
        return self.kind == "table"

    @property
    def parent(self) -> Shape | None:
        """The group this shape belongs to, if any."""
        if self._info.parent is None:
            return None
        return Shape(self.slide, self._n.shape_info(self.slide.id, self._info.parent))

    @property
    def frame(self) -> tuple[int, int, int, int] | None:
        """``(left, top, width, height)`` in EMU, if the shape has its own position."""
        return self._live().frame

    @property
    def text(self) -> str:
        """Text of the shape; paragraphs are separated by ``\\n``, line breaks
        within a paragraph are ``\\v``. Empty for shapes without text."""
        return self._n.shape_text(self.slide.id, self.id) or ""

    @text.setter
    def text(self, value: Any) -> None:
        self._n.set_shape_text(self.slide.id, self.id, _text(value))

    def replace_text(self, replacements: Mapping[str, Any]) -> int:
        """Replaces placeholders in this shape (text and table cells)."""
        return self._n.replace_text(_replacements(replacements), self.slide.id, self.id)

    def replace_image(
        self,
        image: str | os.PathLike[str] | bytes | IO[bytes],
        *,
        fit: Literal["stretch", "contain", "cover"] = "stretch",
    ) -> None:
        """Replaces the image of a picture (or of a shape filled with a
        picture), keeping position, border and effects.

        ``fit`` decides what happens when the aspect ratios differ:
        ``"stretch"`` fills the frame, ``"contain"`` shrinks the frame to
        the image (centred), ``"cover"`` crops the image to fill the frame.
        PNG, JPEG, GIF, BMP, TIFF, EMF and WMF are supported.
        """
        if isinstance(image, (str, os.PathLike)):
            with open(image, "rb") as f:
                data = f.read()
        elif isinstance(image, (bytes, bytearray, memoryview)):
            data = bytes(image)
        else:
            data = image.read()
        self._n.replace_picture(self.slide.id, self.id, data, fit)

    def delete(self) -> None:
        """Removes the shape. Charts and images only it used are dropped on save."""
        self._n.delete_shape(self.slide.id, self.id)

    @property
    def table(self) -> Table:
        if not self.is_table:
            raise UnsupportedError(f"shape {self.name!r} is a {self.kind}, not a table")
        return Table(self)

    @property
    def chart(self) -> Chart:
        if not self.is_chart:
            raise UnsupportedError(f"shape {self.name!r} is a {self.kind}, not a chart")
        return Chart(self)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Shape) and other.slide == self.slide and other.id == self.id

    def __hash__(self) -> int:
        return hash((self.slide, self.id))

    def __repr__(self) -> str:
        return f"<Shape {self.name!r} kind={self.kind} id={self.id}>"


class Table:
    """A table. Cells are addressed as ``table[row, column]``, 0-based."""

    __slots__ = ("shape",)

    def __init__(self, shape: Shape) -> None:
        self.shape = shape

    @property
    def _args(self) -> tuple[int, int]:
        return self.shape.slide.id, self.shape.id

    @property
    def _n(self) -> _native.Presentation:
        return self.shape._n

    @property
    def size(self) -> tuple[int, int]:
        """``(rows, columns)``"""
        return self._n.table_size(*self._args)

    @property
    def rows(self) -> int:
        return self.size[0]

    @property
    def columns(self) -> int:
        return self.size[1]

    @property
    def values(self) -> list[list[str]]:
        return self._n.table_values(*self._args)

    def __getitem__(self, cell: tuple[int, int]) -> str:
        row, col = cell
        return self.values[row][col]

    def __setitem__(self, cell: tuple[int, int], value: Any) -> None:
        row, col = cell
        self._n.set_table_cell_text(*self._args, row, col, _text(value))

    def fill(
        self,
        rows: Iterable[Sequence[Any]],
        *,
        start_row: int = 1,
        resize: bool = True,
    ) -> None:
        """Writes rows of values into the table, starting below the header.

        With ``resize`` (the default) the table grows by copying its last row,
        or shrinks from the end, so it ends exactly after the data. Values are
        converted with ``str()``; ``None`` becomes an empty cell.
        """
        data = [[_text(v) for v in row] for row in rows]
        self._n.fill_table(*self._args, data, start_row, resize)

    def insert_row(self, at: int | None = None, *, copy_of: int = -1) -> None:
        """Inserts an empty copy of row `copy_of` (default: the last row) at
        position `at` (default: the end)."""
        rows = self.rows
        source = copy_of + rows if copy_of < 0 else copy_of
        self._n.insert_table_row(*self._args, source, rows if at is None else at)

    def delete_row(self, row: int) -> None:
        self._n.delete_table_row(*self._args, row + self.rows if row < 0 else row)

    def delete_column(self, column: int) -> None:
        self._n.delete_table_column(*self._args, column + self.columns if column < 0 else column)

    def __repr__(self) -> str:
        rows, cols = self.size
        return f"<Table {self.shape.name!r} {rows}x{cols}>"


class Chart:
    """A native PowerPoint chart."""

    __slots__ = ("shape",)

    def __init__(self, shape: Shape) -> None:
        self.shape = shape

    @property
    def _args(self) -> tuple[int, int]:
        return self.shape.slide.id, self.shape.id

    @property
    def _n(self) -> _native.Presentation:
        return self.shape._n

    @property
    def types(self) -> list[str]:
        """Plot types, e.g. ``["barChart"]`` or ``["barChart", "lineChart"]``."""
        return self._n.chart_types(*self._args)

    @property
    def is_xy(self) -> bool:
        return any(t in ("scatterChart", "bubbleChart") for t in self.types)

    @property
    def categories(self) -> list[str] | list[float] | list[tuple[str, ...]]:
        """Category labels; numbers for numeric and date axes (dates as Excel
        serial numbers); tuples (outermost level first) for multi-level
        categories."""
        categories, kind, _ = self._n.chart_data(*self._args)
        if kind == "levels":
            return [tuple(levels) for levels in categories]  # type: ignore[arg-type]
        return categories  # type: ignore[return-value]

    @property
    def series(self) -> list[Series]:
        """Series in legend order. ``plot`` is set for combo charts only."""
        _, _, series = self._n.chart_data(*self._args)
        return [
            Series(name, list(values), plot, number_format)
            for name, values, plot, number_format in series
        ]

    @property
    def xy_series(self) -> list[XySeries]:
        return [
            XySeries(name, list(x), list(y), None if s is None else list(s))
            for name, x, y, s in self._n.chart_xy_data(*self._args)
        ]

    @property
    def title(self) -> str | None:
        return self._n.chart_title(*self._args)

    @title.setter
    def title(self, value: Any) -> None:
        self._n.set_chart_title(*self._args, _text(value))

    def replace_data(
        self,
        categories: Any = None,
        series: Mapping[str, Sequence[Number]] | Sequence[Series] | None = None,
    ) -> None:
        """Replaces categories and series of a bar, column, line, pie, area,
        doughnut or radar chart.

        ``series`` is either a mapping ``{name: values}`` or a list of
        :class:`Series`. Alternatively pass a pandas DataFrame as the only
        argument: its index becomes the categories, each column a series.

        Categories may be strings, numbers, dates, or tuples for multi-level
        categories (``("2026", "Q1")``, outermost first). Values may be numbers
        or ``None`` (a gap). If there are more series than in the template, the
        extra ones copy the formatting of the template's last series; surplus
        template series are removed. The embedded workbook is rewritten too,
        so "Edit Data" in PowerPoint shows the new numbers.

        In combo charts every plot keeps its own formatting: set
        :attr:`Series.plot` to say which plot each series belongs to. Without
        it, the number of series must match the template.
        """
        if series is None and hasattr(categories, "columns") and hasattr(categories, "index"):
            frame = categories
            categories = list(frame.index)
            series = {str(col): list(frame[col]) for col in frame.columns}
        if categories is None or series is None:
            raise InvalidArgumentError("replace_data needs categories and series")
        if isinstance(series, Mapping):
            items = [Series(str(name), list(values)) for name, values in series.items()]
        else:
            items = list(series)
        native_categories, dates = _categories(categories)
        self._n.set_chart_data(
            *self._args,
            native_categories,
            [(s.name, [_to_float(v) for v in s.values], s.plot, s.number_format) for s in items],
            dates,
        )

    def replace_xy_data(self, series: Sequence[XySeries]) -> None:
        """Replaces the series of a scatter or bubble chart."""
        self._n.set_chart_xy_data(
            *self._args,
            [
                (
                    s.name,
                    [_to_float(v) for v in s.x],
                    [_to_float(v) for v in s.y],
                    None if s.sizes is None else [_to_float(v) for v in s.sizes],
                )
                for s in series
            ],
        )

    def __repr__(self) -> str:
        return f"<Chart {self.shape.name!r} {'/'.join(self.types)}>"
