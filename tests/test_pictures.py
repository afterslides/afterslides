from __future__ import annotations

import io
import re
import struct
import zlib

import pytest

from afterslides import InvalidArgumentError, Presentation, UnsupportedError
from conftest import as_python_pptx, reopen, zip_entries


def png(width: int, height: int, rgb: tuple[int, int, int] = (0, 128, 255)) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    row = b"\x00" + bytes(rgb) * width
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(row * height))
        + chunk(b"IEND", b"")
    )


def media(prs: Presentation) -> dict[str, bytes]:
    return {n: d for n, d in zip_entries(prs).items() if n.startswith("ppt/media/")}


def test_replace_image_swaps_media(prs: Presentation) -> None:
    logo = prs.shape("Logo")
    new = png(40, 20)
    logo.replace_image(new)
    files = media(prs)
    assert list(files.values()) == [new]  # the old image is gone
    check = as_python_pptx(prs).slides[5].shapes[1]
    assert check.image.blob == new
    assert reopen(prs).shape("Logo").frame == logo.frame


def test_contain_keeps_aspect_ratio_inside_old_frame(prs: Presentation) -> None:
    logo = prs.shape("Logo")
    x, y, cx, cy = logo.frame  # type: ignore[misc]
    logo.replace_image(png(40, 20), fit="contain")
    nx, ny, ncx, ncy = reopen(prs).shape("Logo").frame  # type: ignore[misc]
    assert (ncx, ncy) == (cx, cy // 2)
    assert (nx, ny) == (x, y + cy // 4)


def test_cover_crops(prs: Presentation) -> None:
    prs.shape("Logo").replace_image(png(40, 20), fit="cover")
    xml = zip_entries(prs)["ppt/slides/slide6.xml"].decode()
    assert re.search(r'<a:srcRect l="25000" r="25000"/>', xml)
    assert prs.shape("Logo").frame is not None


def test_replace_image_from_path_and_stream(prs: Presentation, tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "logo.png"
    path.write_bytes(png(2, 2))
    prs.shape("Logo").replace_image(path)
    prs.shape("Logo").replace_image(io.BytesIO(png(3, 3)))
    assert list(media(prs).values()) == [png(3, 3)]


def test_shared_image_is_kept_for_other_slides(prs: Presentation) -> None:
    copy = prs.slides[5].duplicate()
    copy.shape("Logo").replace_image(png(5, 5))
    assert len(media(prs)) == 2  # the original slide still uses the old one


def test_replace_image_errors(prs: Presentation) -> None:
    with pytest.raises(InvalidArgumentError):
        prs.shape("Logo").replace_image(b"<svg/>")
    with pytest.raises(InvalidArgumentError):
        prs.shape("Logo").replace_image(png(1, 1), fit="zoom")  # type: ignore[arg-type]
    with pytest.raises(UnsupportedError):
        prs.shape("Note").replace_image(png(1, 1))
