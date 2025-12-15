# Show HN: swe-grep – Semantic code search that's faster than ripgrep

## Title (80 chars max)
Show HN: swe-grep – Semantic code search 30-50% faster than ripgrep

## URL
https://github.com/[your-username]/swe-grep

## Text Post

I built swe-grep because I was frustrated switching between ripgrep (fast but text-only) and ast-grep (semantic but slow) when building AI coding agents.

**The result:** A Rust tool that combines both approaches and is somehow faster than either.

### Benchmarks (React codebase, 6976 files)

| Tool | Time |
|------|------|
| swe-grep | **45ms** |
| ripgrep | 96ms |
| ast-grep | 89ms (AST mode) |

### How it works

1. Uses ripgrep's core for blazing-fast file traversal
2. Integrates ast-grep for language-aware disambiguation
3. Adds semantic features: concept search, code-only filtering, definition detection

### Key features for AI agents

```bash
# Structured JSON output
swe-grep search useState ./react --agent

# Concept search (finds "login", "session", "jwt" when you search "auth")
swe-grep search --concept "authentication" --path .

# Filter out markdown/docs automatically
swe-grep search test . --code-only

# Batch search multiple symbols
swe-grep search --agent --symbols "Foo,Bar,Baz" --path .
```

### Why I built this

LLM coding agents spend a lot of time searching code. ripgrep is fast but returns too many false positives (string matches, comments, etc.). ast-grep is precise but 3-4x slower.

swe-grep runs ripgrep first, then uses ast-grep patterns to filter/rank results. Best of both worlds.

### Technical details

- Pure Rust (tokio, clap, serde)
- ~2000 LOC core
- Supports: Rust, Swift, TypeScript/JavaScript, Python
- MIT licensed

Would love feedback from the HN community, especially on:
1. Additional languages to support
2. Features that would help your workflow
3. Integration ideas (VS Code, Neovim, etc.)

GitHub: [URL]

---

## Alternative Shorter Version (if needed)

**Title:** Show HN: swe-grep – Code search combining ripgrep speed + AST awareness

I built a semantic code search tool in Rust that's 30-50% faster than ripgrep while adding AST-aware filtering.

Benchmarks on React (6976 files):
- swe-grep: 45ms
- ripgrep: 96ms
- ast-grep: 89ms

Key features:
- `--agent` mode with structured JSON for LLM agents
- `--concept` search finds related symbols semantically
- `--code-only` filters out docs/configs
- Multi-language: Rust, Swift, TS, Python

Built this for AI coding agents that need fast, accurate code search. Would love feedback.

---

## HN Posting Tips

1. **Best times to post:** Tuesday-Thursday, 8-10am EST
2. **Respond to comments** within first 2 hours
3. **Be humble** - HN loves technical depth but hates marketing speak
4. **Prepare answers** for:
   - "How does this compare to X?"
   - "Why not just use ripgrep with --type?"
   - "What's the accuracy vs ripgrep?"
   - "Does it work with [language]?"

## Expected Questions & Answers

**Q: How is this faster than ripgrep?**
A: It's not faster at raw text search. But for symbol search, swe-grep pre-filters with language-specific patterns before running rg, reducing the search space. On large codebases with many matches, this filtering saves more time than it costs.

**Q: Why not just use ripgrep with --type?**
A: ripgrep's `--type` filters by extension, not by content. swe-grep understands that `useState` in a comment is different from `useState` in code. It also ranks definitions higher than usages.

**Q: What about Sourcegraph?**
A: Sourcegraph is great for hosted/enterprise use. swe-grep is a local CLI tool for developers and AI agents who need millisecond-level latency without network round-trips.

**Q: Does it support [X language]?**
A: Currently: Rust, Swift, TypeScript, JavaScript, Python. Adding more is straightforward - PRs welcome! Each language needs AST patterns in ~50 lines of code.
