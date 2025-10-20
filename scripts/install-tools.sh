#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to install tool dependencies" >&2
  exit 1
fi

AST_GREP_VERSION="${AST_GREP_VERSION:-}" 
RGA_VERSION="${RGA_VERSION:-}"

echo "Installing ast-grep (sg)..."
if [[ -n "${AST_GREP_VERSION}" ]]; then
  cargo install ast-grep --locked --version "${AST_GREP_VERSION}"
else
  cargo install ast-grep --locked
fi

echo "Installing ripgrep-all (rga)..."
if [[ -n "${RGA_VERSION}" ]]; then
  cargo install ripgrep_all --locked --version "${RGA_VERSION}"
else
  cargo install ripgrep_all --locked
fi

echo "Tool installation complete. Ensure \$HOME/.cargo/bin is on your PATH."
