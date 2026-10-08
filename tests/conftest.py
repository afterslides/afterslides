from __future__ import annotations

import io
import shutil
import subprocess
import zipfile
from pathlib import Path

import pptx
import pytest

import afterslides
from invariants import check_deck

FIXTURES = Path(__file__).parent / "fixtures"
TEMPLATE = FIXTURES / "template.pptx"


@pytest.fixture
def template_path() -> Path:
    return TEMPLATE


@pytest.fixture
def prs() -> afterslides.Presentation:
    return afterslides.Presentation(TEMPLATE)


def saved(deck: afterslides.Presentation) -> bytes:
    """Saves the deck and checks the structural invariants of the result."""
    data = deck.to_bytes()
    check_deck(data)
    return data


def reopen(deck: afterslides.Presentation) -> afterslides.Presentation:
    return afterslides.Presentation(saved(deck))


def as_python_pptx(deck: afterslides.Presentation) -> pptx.presentation.Presentation:
    """Reads our output with python-pptx, an independent implementation."""
    return pptx.Presentation(io.BytesIO(saved(deck)))


def zip_entries(deck: afterslides.Presentation) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(saved(deck))) as z:
        return {name: z.read(name) for name in z.namelist()}


def soffice() -> str | None:
    return shutil.which("soffice") or shutil.which("libreoffice")


def convert_with_libreoffice(pptx_path: Path, out_dir: Path, fmt: str = "pdf") -> Path:
    """Converts a deck with LibreOffice; a failure means it could not read it."""
    binary = soffice()
    assert binary is not None
    subprocess.run(
        [
            binary,
            f"-env:UserInstallation=file://{out_dir / 'profile'}",
            "--headless",
            "--convert-to",
            fmt,
            "--outdir",
            str(out_dir),
            str(pptx_path),
        ],
        check=True,
        capture_output=True,
        timeout=180,
    )
    out = out_dir / f"{pptx_path.stem}.{fmt}"
    assert out.exists() and out.stat().st_size > 0, f"LibreOffice did not produce a {fmt}"
    return out
