"""Rendering prototype: PDF and PNG output."""

from __future__ import annotations

import io
import shutil
import subprocess
from pathlib import Path

import pytest
from PIL import Image

from afterslides import Presentation


def pdf_text(data: bytes, tmp_path: Path) -> str:
    if shutil.which("pdftotext") is None:
        pytest.skip("pdftotext (poppler) is not installed")
    path = tmp_path / "deck.pdf"
    path.write_bytes(data)
    return subprocess.run(
        ["pdftotext", str(path), "-"], check=True, capture_output=True, text=True
    ).stdout


def test_pdf_contains_real_text(prs: Presentation, tmp_path: Path) -> None:
    prs.replace_text({"{{title}}": "Quarterly report", "{{customer}}": "Acme"})
    data = prs.render_pdf()
    assert data.startswith(b"%PDF-")
    text = pdf_text(data, tmp_path)
    assert "Quarterly report" in text  # ligatures map back to their characters
    assert "Report for Acme" in text
    assert "Top customers" in text


def test_pdf_to_path_and_stream(prs: Presentation, tmp_path: Path) -> None:
    path = tmp_path / "out.pdf"
    data = prs.render_pdf(path)
    assert path.read_bytes() == data
    buf = io.BytesIO()
    prs.render_pdf(buf)
    assert buf.getvalue() == data


def test_png_size_follows_scale(prs: Presentation) -> None:
    width, height = (v / 12700 for v in prs.slide_size)
    for scale in (0.5, 2.0):
        image = Image.open(io.BytesIO(prs.slides[0].render_png(scale=scale)))
        assert image.size == (round(width * scale), round(height * scale))


def test_hidden_slides_are_skipped_unless_asked(prs: Presentation, tmp_path: Path) -> None:
    prs.replace_text({"{{title}}": "Visible title"})
    prs.slides[0].hidden = True
    assert "Visible title" not in pdf_text(prs.render_pdf(), tmp_path)
    assert "Visible title" in pdf_text(prs.render_pdf(include_hidden=True), tmp_path)


def test_rendering_reflects_edits(prs: Presentation) -> None:
    before = prs.slides[1].render_png(scale=0.5)
    prs.shape("Revenue Chart").chart.replace_data(["A"], {"x": [100]})
    assert prs.slides[1].render_png(scale=0.5) != before


def test_every_shape_kind_renders(prs: Presentation) -> None:
    # A smoke test across the fixture: pictures, groups, tables, charts.
    for slide in prs.slides:
        image = Image.open(io.BytesIO(slide.render_png(scale=0.25)))
        assert image.getbbox() is not None
