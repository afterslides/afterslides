from __future__ import annotations

import io
import zipfile
from pathlib import Path

import pytest

import afterslides
from afterslides import NotFoundError, PackageError, Presentation
from conftest import TEMPLATE, reopen, zip_entries


def test_open_from_path_bytes_and_stream(template_path: Path) -> None:
    data = template_path.read_bytes()
    for source in (template_path, str(template_path), data, bytearray(data), io.BytesIO(data)):
        assert len(Presentation(source).slides) == 7


def test_untouched_save_is_byte_identical_per_part(prs: Presentation) -> None:
    with zipfile.ZipFile(TEMPLATE) as original:
        expected = {n: original.read(n) for n in original.namelist()}
    assert zip_entries(prs) == expected


def test_save_to_path_and_stream(prs: Presentation, tmp_path: Path) -> None:
    out = tmp_path / "out.pptx"
    prs.save(out)
    assert len(Presentation(out).slides) == 7
    buf = io.BytesIO()
    prs.save(buf)
    assert len(Presentation(buf.getvalue()).slides) == 7


def test_save_over_the_template(tmp_path: Path) -> None:
    path = tmp_path / "deck.pptx"
    path.write_bytes(TEMPLATE.read_bytes())
    deck = Presentation(path)
    deck.replace_text({"{{title}}": "Overwritten"})
    deck.save(path)
    assert "Overwritten" in Presentation(path).slides[0].shapes[0].text
    assert [p.name for p in tmp_path.iterdir()] == ["deck.pptx"]


def test_errors() -> None:
    with pytest.raises(OSError):
        Presentation("/nonexistent/deck.pptx")
    with pytest.raises(PackageError):
        Presentation(b"not a zip file")
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("hello.txt", "hi")
    with pytest.raises(PackageError):
        Presentation(buf.getvalue())
    with pytest.raises(TypeError):
        Presentation(42)  # type: ignore[arg-type]


def test_error_hierarchy() -> None:
    assert issubclass(NotFoundError, LookupError)
    assert issubclass(afterslides.InvalidArgumentError, ValueError)
    assert issubclass(PackageError, afterslides.AfterslidesError)


def test_slides_sequence(prs: Presentation) -> None:
    slides = prs.slides
    assert len(slides) == 7
    assert slides[-1].index == 6
    assert [s.index for s in slides[1:3]] == [1, 2]
    with pytest.raises(IndexError):
        slides[7]
    assert slides[0] == prs.slides[0]
    assert len({slides[0], prs.slides[0]}) == 1


def test_slide_size(prs: Presentation) -> None:
    assert prs.slide_size == (9144000, 6858000)


def test_version() -> None:
    assert afterslides.__version__.count(".") == 2


def test_reopen_helper(prs: Presentation) -> None:
    assert len(reopen(prs).slides) == 7
