"""Compares afterslides' slide rendering with LibreOffice's.

    uv run --no-sync python scripts/render_compare.py [deck.pptx ...] [--out DIR] [--dpi N]

For each deck: LibreOffice converts it to PDF and pdftoppm rasterizes the
pages (the reference); afterslides renders the same slides to PNG. The script
prints the RMSE per slide (0 = identical, in percent of full scale) and
writes a contact sheet per deck with reference, ours and the difference side
by side.

LibreOffice is not PowerPoint: it is a reference for "roughly right", not
ground truth. Both sides must use the same fonts for the numbers to mean
anything (fc-match Calibri should give Carlito).
"""

from __future__ import annotations

import argparse
import io
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageOps, ImageStat

from afterslides import Presentation

ROOT = Path(__file__).resolve().parent.parent


def reference_pages(deck: Path, work: Path, dpi: int) -> list[Image.Image]:
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    if soffice is None:
        sys.exit("LibreOffice (soffice) is needed for reference renderings")
    subprocess.run(
        [
            soffice,
            f"-env:UserInstallation=file://{work / 'profile'}",
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            str(work),
            str(deck),
        ],
        check=True,
        capture_output=True,
        timeout=300,
    )
    pdf = work / f"{deck.stem}.pdf"
    subprocess.run(
        ["pdftoppm", "-r", str(dpi), "-png", str(pdf), str(work / "ref")],
        check=True,
        capture_output=True,
        timeout=300,
    )
    return [Image.open(p).convert("RGB") for p in sorted(work.glob("ref-*.png"))]


def rmse(a: Image.Image, b: Image.Image) -> float:
    if a.size != b.size:
        b = b.resize(a.size)
    diff = ImageChops.difference(a, b)
    rms = ImageStat.Stat(diff).rms
    return 100.0 * (sum(x * x for x in rms) / len(rms)) ** 0.5 / 255.0


def diff_image(a: Image.Image, b: Image.Image) -> Image.Image:
    if a.size != b.size:
        b = b.resize(a.size)
    diff = ImageOps.invert(ImageOps.autocontrast(ImageChops.difference(a, b).convert("L")))
    return diff.convert("RGB")


def contact_sheet(rows: list[tuple[str, Image.Image, Image.Image]]) -> Image.Image:
    thumb_w = 480
    cells = []
    for label, ref, ours in rows:
        scale = thumb_w / ref.width
        size = (thumb_w, int(ref.height * scale))
        cells.append(
            (label, ref.resize(size), ours.resize(size), diff_image(ref, ours).resize(size))
        )
    pad, header = 8, 22
    height = sum(c[1].height + header + pad for c in cells) + pad
    sheet = Image.new("RGB", (3 * thumb_w + 4 * pad, height), "white")
    draw = ImageDraw.Draw(sheet)
    y = pad
    for label, ref, ours, diff in cells:
        draw.text((pad, y), label, fill="black")
        y += header
        for i, img in enumerate((ref, ours, diff)):
            sheet.paste(img, (pad + i * (thumb_w + pad), y))
        y += ref.height + pad
    return sheet


def compare(deck: Path, out: Path, dpi: int) -> list[dict[str, object]]:
    prs = Presentation(deck)
    visible = [s for s in prs.slides if not s.hidden]
    with tempfile.TemporaryDirectory() as tmp:
        refs = reference_pages(deck, Path(tmp), dpi)
    results = []
    rows = []
    for slide, ref in zip(visible, refs):
        ours = Image.open(io.BytesIO(slide.render_png(scale=dpi / 72))).convert("RGB")
        score = rmse(ref, ours)
        results.append({"deck": deck.name, "slide": slide.index, "rmse": round(score, 2)})
        rows.append((f"{deck.name} slide {slide.index}: RMSE {score:.2f}%", ref, ours))
    if len(refs) != len(visible):
        print(f"  warning: LibreOffice produced {len(refs)} pages for {len(visible)} slides")
    if rows:
        out.mkdir(parents=True, exist_ok=True)
        contact_sheet(rows).save(out / f"{deck.stem}.png")
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("decks", nargs="*", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "render-compare")
    parser.add_argument("--dpi", type=int, default=72)
    args = parser.parse_args()
    decks = args.decks or [ROOT / "tests" / "fixtures" / "template.pptx"]

    all_results = []
    for deck in decks:
        results = compare(deck, args.out, args.dpi)
        all_results += results
        for r in results:
            print(f"{r['deck']:40.40} slide {r['slide']:>2}  RMSE {r['rmse']:6.2f}%")
    if all_results:
        mean = sum(float(r["rmse"]) for r in all_results) / len(all_results)
        print(f"\n{len(all_results)} slides, mean RMSE {mean:.2f}%  (sheets in {args.out})")
        (args.out / "results.json").write_text(json.dumps(all_results, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
