#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

inbox_dir="${FEEDBACK_INBOX_DIR:-Feedback/inbox}"
processed_dir="${FEEDBACK_PROCESSED_DIR:-Feedback/processed}"
failed_dir="${FEEDBACK_FAILED_DIR:-Feedback/failed}"
runs_dir="${FEEDBACK_RUNS_DIR:-Feedback/runs}"

codex_bin="${CODEX_BIN:-codex}"
codex_model="${CODEX_MODEL:-}"
codex_sandbox="${CODEX_SANDBOX:-workspace-write}"
codex_approval="${CODEX_APPROVAL_POLICY:-never}"

open_pr="${OPEN_PR:-0}"
pr_base="${PR_BASE_BRANCH:-master}"
pr_remote="${PR_REMOTE:-origin}"

require_clean="${REQUIRE_CLEAN_WORKTREE:-1}"

usage() {
  cat <<'EOF'
Usage:
  scripts/feedback/process_inbox.sh

Environment:
  FEEDBACK_INBOX_DIR=Feedback/inbox
  FEEDBACK_PROCESSED_DIR=Feedback/processed
  FEEDBACK_FAILED_DIR=Feedback/failed
  FEEDBACK_RUNS_DIR=Feedback/runs

  CODEX_BIN=codex
  CODEX_MODEL=o3 (optional)
  CODEX_SANDBOX=workspace-write
  CODEX_APPROVAL_POLICY=never

  OPEN_PR=0|1 (optional, requires gh auth)
  PR_BASE_BRANCH=master
  PR_REMOTE=origin
  REQUIRE_CLEAN_WORKTREE=1|0
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if ! command -v jq >/dev/null; then
  echo "[feedback] jq is required" >&2
  exit 2
fi

if [[ "$require_clean" == "1" ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    echo "[feedback] working tree is dirty; commit/stash first (or set REQUIRE_CLEAN_WORKTREE=0)" >&2
    exit 2
  fi
fi

mkdir -p "$inbox_dir" "$processed_dir" "$failed_dir" "$runs_dir"

shopt -s nullglob
records=("$inbox_dir"/*.json)
shopt -u nullglob

if [[ "${#records[@]}" -eq 0 ]]; then
  echo "[feedback] no records in $inbox_dir"
  exit 0
fi

for record_path in "${records[@]}"; do
  file_base="$(basename "$record_path")"
  id="$(jq -r '.id // empty' "$record_path" 2>/dev/null || true)"
  if [[ -z "$id" ]]; then
    id="${file_base%.json}"
  fi

  slug="$(printf '%s' "$id" | tr -cs '[:alnum:]._-' '-' | tr '[:upper:]' '[:lower:]' | sed 's/^-//;s/-$//')"
  if [[ -z "$slug" ]]; then
    slug="record-$(date +%s)"
  fi

  branch="feedback/$slug"
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    branch="feedback/$slug-$(date +%s)"
  fi

  echo "[feedback] processing $record_path (branch $branch)"
  git checkout -b "$branch"

  if ! jq -e '.schema_version and .id and .invocation and .feedback and .feedback.improvements' "$record_path" >/dev/null; then
    echo "[feedback] invalid record (archiving to $failed_dir): $record_path" >&2
    failed_path="$failed_dir/$file_base"
    mv "$record_path" "$failed_path"
    git add -A -- "$record_path" "$failed_path"
    git -c user.name="codex" -c user.email="codex@users.noreply.github.com" commit -m "Quarantine invalid feedback record $slug" || true
    git checkout "$pr_base" >/dev/null 2>&1 || git checkout master >/dev/null 2>&1 || true
    continue
  fi

  schema_version="$(jq -r '.schema_version' "$record_path")"
  tool="$(jq -r '.invocation.tool // "unknown"' "$record_path")"
  command="$(jq -r '.invocation.command // "unknown"' "$record_path")"
  improvements="$(jq -r '.feedback.improvements' "$record_path")"
  tags="$(jq -r '.feedback.tags // [] | join(",")' "$record_path")"

  run_dir="$runs_dir/$id"
  mkdir -p "$run_dir"

  prompt_file="$run_dir/prompt.md"
  last_message="$run_dir/codex_last_message.txt"

  cat >"$prompt_file" <<EOF
You are running inside the \`swe-grep\` repository.

Goal: implement the following feedback record and ensure all verification passes.

Feedback record:
- schema_version: $schema_version
- id: $id
- tool: $tool
- command: $command
- tags: ${tags:-"(none)"}

Human improvements:
$improvements

Hard requirements:
1) Use \`swe-grep\` for ALL code search (no grep/rg/find).
2) Keep changes minimal, focused, and high-quality.
3) After changes: \`./scripts/feedback/verify.sh\` must pass.
4) If the feedback requests something that cannot be implemented in this repo, add a short note to \`$run_dir/unimplemented.md\` explaining why, and still ensure \`./scripts/feedback/verify.sh\` passes.

When finished, leave the working tree with the necessary code changes applied.
EOF

  declare -a codex_args=()
  if [[ -n "$codex_model" ]]; then
    codex_args+=(-m "$codex_model")
  fi

  "$codex_bin" -s "$codex_sandbox" -a "$codex_approval" exec -C "$repo_root" ${codex_args[@]+"${codex_args[@]}"} --output-last-message "$last_message" - <"$prompt_file"

  echo "[feedback] verify"
  ./scripts/feedback/verify.sh

  echo "[feedback] commit changes"
  if [[ -n "$(git status --porcelain)" ]]; then
    git add -A -- . \
      ":!$runs_dir/**" \
      ":!$inbox_dir/shell_feedback.ndjson"
    git -c user.name="codex" -c user.email="codex@users.noreply.github.com" commit -m "Apply feedback $id"
  fi

  echo "[feedback] mark record processed"
  processed_path="$processed_dir/$file_base"
  mv "$record_path" "$processed_path"
  git add -A -- "$record_path" "$processed_path"
  git -c user.name="codex" -c user.email="codex@users.noreply.github.com" commit -m "Archive feedback record $id" || true

  if [[ "$open_pr" == "1" ]]; then
    if command -v gh >/dev/null; then
      echo "[feedback] pushing branch + opening PR"
      git push -u "$pr_remote" "$branch"
      gh pr create --fill --base "$pr_base" --head "$branch" || true
    else
      echo "[feedback] OPEN_PR=1 but gh not found; skipping PR creation" >&2
    fi
  else
    echo "[feedback] done (branch $branch)"
  fi

  git checkout "$pr_base" >/dev/null 2>&1 || git checkout master >/dev/null 2>&1 || true
done
