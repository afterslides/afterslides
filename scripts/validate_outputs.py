"""Validates every deck the test suite dumped with the Open XML SDK.

    AFTERSLIDES_DUMP_DIR=out pytest ...
    python scripts/validate_outputs.py out

Errors the input deck already had (same error id in the same kind of part)
are ignored: we only answer for the problems we introduce. Exits with 1 if
there are new errors.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOL = ROOT / "tools" / "ooxml-validate"


def validate(paths: list[Path]) -> dict[str, set[tuple[str, str]]]:
    """Maps file name -> {(part kind, error id)}."""
    result = subprocess.run(
        ["dotnet", "run", "--project", str(TOOL), "-c", "Release", "--", "--tsv", *map(str, paths)],
        capture_output=True,
        text=True,
    )
    if result.returncode not in (0, 1):
        sys.exit(f"validator failed:\n{result.stderr}")
    errors: dict[str, set[tuple[str, str]]] = defaultdict(set)
    for line in result.stdout.splitlines():
        file, part, error_id, description = line.split("\t", 3)
        kind = re.sub(r"\d+", "#", part)
        errors[str(Path(file).resolve())].add((kind, error_id, description))
    return errors


def main() -> int:
    dump_dir = Path(sys.argv[1]).resolve()
    manifest = {}
    for line in (dump_dir / "manifest.tsv").read_text(encoding="utf-8").splitlines():
        out, source = line.split("\t")
        manifest[str(dump_dir / out)] = source
    sources = sorted({Path(s) for s in manifest.values()})

    out_errors = validate([dump_dir])
    in_errors = validate(sources)

    new = 0
    for out, errors in sorted(out_errors.items()):
        known = {(k, i) for k, i, _ in in_errors.get(manifest.get(out, ""), set())}
        fresh = sorted(e for e in errors if (e[0], e[1]) not in known)
        if fresh:
            new += 1
            print(f"{Path(out).name}  (from {Path(manifest.get(out, '?')).name})")
            for kind, error_id, description in fresh[:5]:
                print(f"    {kind} [{error_id}] {description[:200]}")
    print(f"{len(manifest)} decks validated, {new} with new errors", file=sys.stderr)
    return 1 if new else 0


if __name__ == "__main__":
    sys.exit(main())
