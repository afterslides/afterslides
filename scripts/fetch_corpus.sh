#!/usr/bin/env bash
# Downloads the real-world test corpus into .corpus/ (or $1).
#
# The decks come from the test suites of other open source OOXML projects,
# pinned to fixed commits so test runs are reproducible. They are not
# committed here; see docs/testing.md for sources and licenses.
set -euo pipefail
cd "$(dirname "$0")/.."
dest="${1:-.corpus}"

fetch() { # name repo commit path
  local name=$1 repo=$2 commit=$3 path=$4
  if [[ -f "$dest/$name/.commit" && "$(cat "$dest/$name/.commit")" == "$commit" ]]; then
    echo "$name: up to date"
    return
  fi
  local tmp
  tmp="$(mktemp -d)"
  git -C "$tmp" init -q
  git -C "$tmp" remote add origin "$repo"
  git -C "$tmp" sparse-checkout set --no-cone "/$path/*.pptx"
  git -C "$tmp" fetch -q --depth 1 --filter=blob:none origin "$commit"
  git -C "$tmp" checkout -q FETCH_HEAD
  rm -rf "${dest:?}/$name"
  mkdir -p "$dest/$name"
  cp "$tmp/$path"/*.pptx "$dest/$name/"
  echo "$commit" > "$dest/$name/.commit"
  rm -rf "$tmp"
  echo "$name: $(ls "$dest/$name"/*.pptx | wc -l) decks"
}

# MIT License, Copyright (c) 2013 Steve Canny
fetch python-pptx https://github.com/scanny/python-pptx.git \
  278b47b1dedd5b46ee84c286e77cdfb0bf4594be features/steps/test_files

# Apache License 2.0, The Apache Software Foundation
fetch apache-poi https://github.com/apache/poi.git \
  ae62bb5116b9aee19ebd5834e3a82066132c9f7f test-data/slideshow
