# Feedback

This folder stores structured feedback artifacts collected via `swe-grep feedback …`.

## Automated feedback → fix → verify workflow

This repo supports an automated pipeline driven by feedback records:

- Drop new records into `Feedback/inbox/` (one JSON file per record).
- Tip: write directly to the inbox with `swe-grep feedback shell … --out-dir Feedback/inbox`.
- Run `./scripts/feedback/process_inbox.sh` to:
  - spin up a branch per record,
  - run Codex to implement the suggestion,
  - run `./scripts/feedback/verify.sh` (tests + release build + smoke),
  - move the record to `Feedback/processed/` on success (or `Feedback/failed/` if invalid).

If you enable GitHub Actions, `.github/workflows/feedback-autofix.yml` will do the same on pushes that add files under `Feedback/inbox/` (requires `OPENAI_API_KEY` secret).

## Collecting shell tool feedback

### 1) Ask the agent for feedback (prompt only)

```bash
swe-grep feedback shell --command "ls -la" --prompt-only
```

### 2) Save feedback (pipe JSON or plain text)

```bash
echo '{"improvements":"Add stdout/stderr streaming + stable truncation.","rating":4,"tags":["streaming","ux"]}' \
  | swe-grep feedback shell --command "ls -la" --cwd "$(pwd)" --exit-code 0 --duration-ms 12
```

### Output files

- `shell_feedback_<id>.json`: one JSON object per feedback record (easy to review, safe to move).
- `shell_feedback.ndjson`: append-only JSON Lines stream (easy to ingest at scale).

## Conventions

- Default output dir is `<cwd>/Feedback` when `--cwd` is provided; otherwise it is `./Feedback` (override with `--out-dir`).
- Schema is versioned (`schema_version`) so ingestion can evolve safely.
- Prefer piping JSON (as requested by the prompt) for best downstream analytics.
