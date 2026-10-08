from __future__ import annotations

import io
import shutil
import subprocess
import zipfile
from pathlib import Path

import pptx
import pytest

import afterslides

FIXTURES = Path(__file__).parent / "fixtures"
TEMPLATE = FIXTURES / "template.pptx"


@pytest.fixture
def template_path() -> Path:
    return TEMPLATE


@pytest.fixture
def prs() -> afterslides.Presentation:
    return afterslides.Presentation(TEMPLATE)


def reopen(deck: afterslides.Presentation) -> afterslides.Presentation:
    return afterslides.Presentation(deck.to_bytes())


def as_python_pptx(deck: afterslides.Presentation) -> pptx.presentation.Presentation:
    """Reads our output with python-pptx, an independent implementation."""
    return pptx.Presentation(io.BytesIO(deck.to_bytes()))


def zip_entries(deck: afterslides.Presentation) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(deck.to_bytes())) as z:
        return {name: z.read(name) for name in z.namelist()}


def soffice() -> str | None:
    return shutil.which("soffice") or shutil.which("libreoffice")


def convert_with_libreoffice(pptx_path: Path, out_dir: Path) -> Path:
    """Converts a deck to PDF; a failure means LibreOffice could not read it."""
    binary = soffice()
    assert binary is not None
    subprocess.run(
        [
            binary,
            f"-env:UserInstallation=file://{out_dir / 'profile'}",
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            str(out_dir),
            str(pptx_path),
        ],
        check=True,
        capture_output=True,
        timeout=180,
    )
    pdf = out_dir / (pptx_path.stem + ".pdf")
    assert pdf.exists(), "LibreOffice did not produce a PDF"
    return pdf
