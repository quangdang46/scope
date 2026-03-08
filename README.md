# scope

> Instant dependency graph for any file or function. Know the full impact of a change **before** making it.

---

## The Problem

Before editing a file, Claude Code needs to understand:
- What does this file import?
- Who imports this file?
- Which functions are public vs internal?
- If I change this, what else breaks?

Without a tool, Claude Code reads file after file, following import chains manually. That's 5–20 file reads per question, burning context window before any real work begins.

## Why scope Is Better

| | Claude Code alone | scope |
|---|---|---|
| Find all importers of a file | Read N files manually | Instant from index |
| Map function call graph | Read + parse manually | Pre-built graph |
| Estimate change impact | Guess or exhaustive read | `scope impact <file>` |
| Token cost | High (many file reads) | Near zero (structured JSON) |
| Speed | Seconds to minutes | < 100ms |

## How It Works

```
1. Index phase  (once, ~2s for 10k file project)
   └── walk all source files (respects .gitignore)
   └── tree-sitter extracts: imports, exports, function defs, calls
   └── build directed graph: file → file, function → function
   └── persist as sqlite

2. Query phase  (instant)
   └── graph traversal: O(1) lookups by file/function name
   └── output as JSON (Claude reads) or pretty-print (human reads)
```

**Zero LLM calls. Zero API. Pure static analysis.**

## Tech Stack

| Crate | Purpose |
|---|---|
| `tree-sitter` + grammars | Parse imports/exports/calls per language |
| `petgraph` | Directed dependency graph (DFS, BFS, cycle detection) |
| `rusqlite` | Persist graph between runs |
| `ignore` | Gitignore-aware file walking |
| `clap` | CLI |
| `serde_json` | Structured output for Claude Code |

## Supported Languages

- JavaScript / TypeScript / JSX / TSX
- Rust
- Python
- Ruby
- Go

## Installation

```bash
cargo install scope-cli
```

## Usage

```bash
# Build dependency graph (run once)
scope index .

# Who imports this file? Who does it import?
scope file src/auth/middleware.js

# Where is this function called?
scope fn verifyToken

# If I change this file, what's the blast radius?
scope impact src/auth/middleware.js

# Full dependency tree (recursive)
scope tree src/auth/middleware.js --depth 3
```

## Output Examples

### `scope file src/auth/middleware.js`

```json
{
  "file": "src/auth/middleware.js",
  "imports": [
    "src/models/User.js",
    "src/utils/jwt.js",
    "src/config/env.js"
  ],
  "imported_by": [
    "src/routes/api.js",
    "src/routes/admin.js",
    "src/server.js"
  ],
  "exports": ["verifyToken", "requireAdmin", "optionalAuth"],
  "internal_functions": ["decodePayload", "checkExpiry"]
}
```

### `scope impact src/utils/jwt.js`

```json
{
  "file": "src/utils/jwt.js",
  "direct_dependents": ["src/auth/middleware.js"],
  "transitive_dependents": [
    "src/routes/api.js",
    "src/routes/admin.js",
    "src/server.js",
    "tests/auth.test.js"
  ],
  "risk": "HIGH",
  "reason": "4 files depend on this directly or transitively"
}
```

### `scope fn verifyToken`

```json
{
  "function": "verifyToken",
  "defined_in": "src/auth/middleware.js:14",
  "called_by": [
    "src/routes/api.js:23",
    "src/routes/admin.js:11",
    "tests/auth.test.js:45"
  ],
  "calls": ["jwt.verify", "checkExpiry"]
}
```

## Integration with Claude Code

Add to your project's `CLAUDE.md`:

```markdown
## Custom Tools

- `scope file <path>` — show what a file imports and what imports it
- `scope fn <name>` — find all call sites of a function
- `scope impact <path>` — estimate blast radius before editing

Use these BEFORE reading files to plan which files actually need reading.
```

Claude Code will use `scope` to orient itself before making changes, saving 10–30 file reads per task.

## Index Location

```
your-project/
└── .scope/
    ├── graph.db        # dependency graph
    └── metadata.json   # index timestamp per file
```

Add `.scope/` to `.gitignore`.

---

## Roadmap

- [ ] Cycle detection and circular dependency warnings
- [ ] `scope unused` — find exported symbols never imported anywhere
- [ ] `scope diff <branch>` — show which dependents are affected by branch changes
- [ ] Language server integration (LSP)
