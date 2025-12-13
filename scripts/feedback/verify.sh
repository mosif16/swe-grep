#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

echo "[verify] cargo test"
cargo test --workspace --locked

echo "[verify] cargo build --release"
cargo build --release --locked

bin="$repo_root/target/release/swe-grep"
if [[ ! -x "$bin" ]]; then
  echo "[verify] expected binary at $bin" >&2
  exit 1
fi

echo "[verify] smoke: --help"
"$bin" --help >/dev/null

echo "[verify] smoke: search"
"$bin" search --symbol "ShellFeedbackRecord" --language rust --path crates/swe-grep-core/src --max-matches 1 >/dev/null

echo "[verify] ok"
