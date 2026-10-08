#!/usr/bin/env bash
# Runs the same checks as CI. Use before pushing:  scripts/check.sh [--quick]
set -euo pipefail
cd "$(dirname "$0")/.."

export PYO3_PYTHON="${PYO3_PYTHON:-$PWD/.venv/bin/python}"
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt";    cargo fmt --all --check
step "cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
step "cargo test";   cargo test -p afterslides
step "cargo doc";    RUSTDOCFLAGS="-D warnings" cargo doc -p afterslides --no-deps --quiet
step "ruff";         uvx ruff@0.16.10 check && uvx ruff@0.16.10 format --check
step "mypy";         uvx mypy --strict python/afterslides --ignore-missing-imports
step "build";        uv run --no-sync maturin develop --quiet
if [[ "${1:-}" == "--quick" ]]; then
  step "pytest (quick)"; uv run --no-sync pytest -q -m "not slow and not libreoffice"
else
  step "pytest";         uv run --no-sync pytest -q
fi
if command -v dotnet >/dev/null; then
  step "Open XML SDK validation"
  dump="$(mktemp -d)"
  AFTERSLIDES_DUMP_DIR="$dump" uv run --no-sync pytest -q -m "not slow and not libreoffice" >/dev/null
  cp tests/fixtures/*.pptx "$dump/"
  dotnet run --project tools/ooxml-validate -c Release -- "$dump"
  rm -rf "$dump"
fi
printf '\n\033[32mAll checks passed.\033[0m\n'
