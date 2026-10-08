# Contributing to afterslides

Thanks for helping! Bug reports, templates that break, documentation fixes and
code are all welcome.

## Reporting bugs

The most useful bug report contains a **template that shows the problem**.
Please remove confidential content first (replace texts, numbers and images;
the structure is what matters). Say what you did, what you expected and what
PowerPoint (or LibreOffice, Keynote, Google Slides) shows: an error message,
a repair prompt, wrong data.

## Development setup

You need Rust (stable, 1.88 or newer), [uv](https://docs.astral.sh/uv/) and,
optionally, LibreOffice for the interop tests.

```sh
git clone git@github.com:afterslides/afterslides.git
cd afterslides
uv venv
uv pip install maturin pytest python-pptx openpyxl ruff mypy
export PYO3_PYTHON=$PWD/.venv/bin/python

uv run --no-sync maturin develop       # build the extension into .venv
uv run --no-sync pytest -m "not slow"  # quick test run
cargo test -p afterslides              # Rust tests
```

Before sending a pull request:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p afterslides
uv run --no-sync pytest
uvx ruff check && uvx ruff format --check
uvx mypy --strict python/afterslides --ignore-missing-imports
```

CI runs the same checks on Linux, macOS and Windows.

## How the code is organized

See [AGENTS.md](AGENTS.md) for the layout, the rules every change must follow
(round-trip fidelity, no memory leaks, ...) and the OOXML pitfalls we already
ran into, and [docs/architecture.md](docs/architecture.md) for the design.

## Tests

- New behaviour needs a test. Prefer tests that read our output back with an
  independent implementation (python-pptx, openpyxl, LibreOffice) over tests
  that compare our own XML.
- If you need a template feature the fixture lacks, extend
  `scripts/make_fixtures.py` and regenerate `tests/fixtures/template.pptx`.
- Bugs found with real templates: add the (anonymized) template to
  `tests/fixtures/` together with a regression test.

## Commits and pull requests

- Small, focused commits. Subject line in the imperative ("Add ...",
  "Fix ..."), at most 72 characters; the body explains *why*.
- Update `CHANGELOG.md` under *Unreleased* for user-visible changes.
- Public API changes need docstrings and an update to `README.md` or `docs/`.

## License

By contributing you agree that your contributions are licensed under the
LGPL-3.0-or-later, like the rest of the project.
