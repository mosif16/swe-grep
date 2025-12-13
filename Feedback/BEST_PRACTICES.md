# Best practices for feedback ingestion

## 1) Keep it structured

- Prefer JSON records over free-form text; define a stable schema with explicit versioning.
- Separate **event metadata** (tool, command, duration, exit code) from **human feedback** (improvements, rating, tags).
- Treat feedback as append-only events; avoid in-place mutation.

## 2) Protect sensitive data

- Assume commands and outputs may contain secrets (tokens, API keys, paths, customer data).
- Redact aggressively at the edge (before writing) and again at ingestion (defense in depth).
- Store only what you need; avoid capturing full stdout/stderr unless explicitly required.

## 3) Make ingestion reliable

- Validate JSON on ingest; quarantine invalid records rather than dropping them silently.
- Use idempotent identifiers (`id`) and de-duplication (e.g., “already seen” set).
- Keep ingestion tolerant of partial writes (especially for NDJSON): handle truncated last lines.

## 4) Make analysis trustworthy

- Capture enough context for actionability: exit code, duration, cwd, tool name, and tags.
- Keep “rating” optional; don’t force it (missing data is better than noisy data).
- Normalize tags (lowercase, trim, controlled vocabulary where possible).

## 5) Operate it like a system

- Rotate large NDJSON files and/or shard by date to keep file sizes manageable.
- Restrict access to feedback stores; treat them as production telemetry.
- Document retention and deletion policy (especially if any user data can appear).

## 6) Close the loop

- Aggregate by tags and failure modes to prioritize improvements.
- Track changes with “before/after” metrics (e.g., fewer retries, less output truncation pain).
- Feed insights back into tool design and prompt templates.

