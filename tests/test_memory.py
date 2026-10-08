"""Guards against memory leaks in the native extension.

A report service opens and fills thousands of decks over its lifetime, so
memory must return to a flat line after warm-up.
"""

from __future__ import annotations

import gc
import os
import sys
from pathlib import Path

import pytest

from afterslides import Presentation, Series
from conftest import TEMPLATE


def rss_bytes() -> int:
    with open("/proc/self/statm") as f:
        return int(f.read().split()[1]) * os.sysconf("SC_PAGE_SIZE")


def cycle(data: bytes, out: Path) -> None:
    prs = Presentation(data)
    prs.replace_text({"{{title}}": "x" * 100})
    prs.shape("Revenue Chart").chart.replace_data(
        [f"c{i}" for i in range(50)], [Series(f"s{j}", list(range(50))) for j in range(5)]
    )
    prs.shape("Top Customers").table.fill([[i, i, i] for i in range(30)])
    prs.slides[1].duplicate()
    prs.slides[3].delete()
    prs.to_bytes()
    prs.save(out)


@pytest.mark.slow
@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="reads /proc")
def test_no_growth_over_many_cycles(tmp_path: Path) -> None:
    data = TEMPLATE.read_bytes()
    out = tmp_path / "out.pptx"
    for _ in range(20):
        cycle(data, out)
    gc.collect()
    baseline = rss_bytes()
    for _ in range(200):
        cycle(data, out)
    gc.collect()
    growth = rss_bytes() - baseline
    # One cycle allocates a few MB; a leak of even 20 KB per cycle shows up here.
    assert growth < 6 * 1024 * 1024, f"RSS grew by {growth / 1024 / 1024:.1f} MiB"
