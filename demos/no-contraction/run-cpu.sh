#!/bin/sh
set -eu
expect=${1:-lost}
case "$expect" in lost|preserved) ;; *) echo 'Use lost or preserved' >&2; exit 2;; esac
cd "$(dirname "$0")"
out=${2:-results-$(date -u +%Y%m%dT%H%M%SZ)-$$}
mkdir "$out"
cargo run --locked --manifest-path Cargo.toml -- fixtures/original-controlled.spv fixtures/original-unmarked.spv "$out/naga" "$expect"
