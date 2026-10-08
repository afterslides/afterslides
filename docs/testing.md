# Testing

afterslides edits files that other programs have to open without complaint,
so most tests check our output with tools we didn't write.

## Layers

| Layer | Where | What it catches |
| --- | --- | --- |
| Rust unit and integration tests | `crates/afterslides` | logic of the core, lossless round-trip |
| Python API tests | `tests/test_*.py` | behaviour, read back with python-pptx and openpyxl |
| Invariant checks | `tests/invariants.py` | what PowerPoint is strict about but lenient readers accept: dangling references, missing content types, duplicate ids, broken table spans, animations pointing at missing shapes |
| Regression tests | `tests/test_regressions.py` | constructs from real templates, patched into the fixture |
| Real-world corpus | `tests/test_corpus.py` | every edit on ~150 decks saved by PowerPoint, LibreOffice and others |
| Open XML SDK validation | `tools/ooxml-validate`, `scripts/validate_outputs.py` | schema and semantic errors Office would report |
| LibreOffice interop | `tests/test_libreoffice.py` | another office suite opens and renders the result |
| Memory soak | `tests/test_memory.py` | leaks over hundreds of open/fill/save cycles |

Every deck saved through the helpers in `tests/conftest.py` passes the
invariant checks. With `AFTERSLIDES_DUMP_DIR` set, the decks are also written
to that directory (with a `manifest.tsv` naming each input) so they can be
validated with the Open XML SDK afterwards.

## Running

```sh
scripts/check.sh --quick            # what CI runs, minus slow and LibreOffice tests
scripts/fetch_corpus.sh             # download the corpus into .corpus/ (~30 MB)
uv run --no-sync pytest -m corpus   # corpus only

AFTERSLIDES_DUMP_DIR=/tmp/decks uv run --no-sync pytest -m "not slow"
uv run --no-sync python scripts/validate_outputs.py /tmp/decks   # needs dotnet
```

Real-world files come with their own defects (negative chart axis ids,
short table rows, ...). Corpus tests and `validate_outputs.py` compare each
result with its input and only report problems we introduced.

## Corpus sources and licenses

The corpus is downloaded at pinned commits and not stored in this
repository.

| Source | Files | License |
| --- | --- | --- |
| [python-pptx](https://github.com/scanny/python-pptx) `features/steps/test_files` | 60 decks, mostly saved by PowerPoint | MIT, © Steve Canny |
| [Apache POI](https://github.com/apache/poi) `test-data/slideshow` | 95 decks from bug reports, including fuzzer output | Apache-2.0, © The Apache Software Foundation |

Have a template that afterslides breaks and that you're allowed to share?
Anonymize it and open an issue; it will become a regression test.
