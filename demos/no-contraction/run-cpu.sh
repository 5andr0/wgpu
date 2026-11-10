#!/bin/sh
set -eu
expect=${1:-lost}
case "$expect" in lost|preserved) ;; *) echo 'Use lost or preserved' >&2; exit 2;; esac
cd "$(dirname "$0")"
out=${2:-results-$(date -u +%Y%m%dT%H%M%SZ)-$$}
mkdir "$out"
compiler=${GLSLANG:-glslang}
"$compiler" -V controlled.comp -o "$out/controlled.spv"
"$compiler" -V unmarked.comp -o "$out/unmarked.spv"
cargo run --locked --manifest-path Cargo.toml -- "$out/controlled.spv" "$out/unmarked.spv" "$out/naga" "$expect"
