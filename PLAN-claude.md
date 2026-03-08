# PLAN: scope

## Overview

Instant dependency graph CLI for any codebase.
Know exactly what imports what, who calls what, and the full blast radius of any
change — before making it. Pure static analysis, zero LLM, zero API.

---

## Why Workspace + `crates/` (not `src/`)

- Workspace root holds config, docs, poc, and tests — no source code
- Each concern (parsing, graph, storage) lives in its own independently testable crate
- `src/` exists only inside each sub-crate, never at the root
- Follows the same layout as ripgrep, fd, and the other tools in this suite

---

## File Structure

```
scope/
├── Cargo.toml              # workspace root — no source code here
├── Cargo.lock
├── README.md
├── POC.md
├── PLAN.md
├── .gitignore              # includes .scope/
│
├── crates/
│   ├── core/               # binary entry point
│   │   ├── Cargo.toml
│   │   └── main.rs         # CLI: index / file / fn / impact / tree / unused / cycles
│   │
│   ├── parser/             # tree-sitter extraction per language
│   │   ├── Cargo.toml
│   │   ├── lib.rs
│   │   ├── extractor.rs    # imports / exports / fn defs / call sites
│   │   ├── resolver.rs     # resolve relative import paths to absolute
│   │   └── languages.rs    # language dispatch — no mod.rs (Rust 1.30+ style)
│   │   └── languages/
│   │       ├── javascript.rs
│   │       ├── typescript.rs
│   │       ├── rust.rs
│   │       ├── python.rs
│   │       └── ruby.rs
│   │
│   ├── graph/              # petgraph build + traversal queries
│   │   ├── Cargo.toml
│   │   ├── lib.rs
│   │   ├── builder.rs      # build DiGraph from parsed data
│   │   └── queries.rs      # file / fn / impact BFS / tree DFS / cycles
│   │
│   └── store/              # sqlite persistence
│       ├── Cargo.toml
│       ├── lib.rs
│       └── db.rs           # serialize graph as edge list, mtime tracking
│
├── poc/
│   ├── package.json
│   └── index.js            # Node.js POC
│
├── test-fixtures/
│   ├── utils/
│   │   └── jwt.js
│   ├── middleware/
│   │   └── auth.js
│   └── routes/
│       └── api.js
│
├── tests/
│   └── graph_test.rs       # integration: index → impact query round-trip
│
└── benches/
    └── index_bench.rs
```

### Root `Cargo.toml`

```toml
[workspace]
members = [
    "crates/core",
    "crates/parser",
    "crates/graph",
    "crates/store",
]
resolver = "2"

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"

[[bin]]
name = "scope"
path = "crates/core/main.rs"

[workspace.dependencies]
tree-sitter = "0.22"
tree-sitter-javascript = "0.21"
tree-sitter-typescript = "0.21"
tree-sitter-rust = "0.21"
tree-sitter-python = "0.21"
tree-sitter-ruby = "0.21"
petgraph = "0.6"
rusqlite = { version = "0.31", features = ["bundled"] }
ignore = "0.4"
clap = { version = "4", features = ["derive"] }
serde_json = "1"
serde = { version = "1", features = ["derive"] }
anyhow = "1"
```

---

## Phases

### Phase 1 — POC (Node.js, 1 day)

Validate graph extraction accuracy before writing Rust.

- [ ] tree-sitter extract imports, exports, fn defs, and call sites from JS/TS
- [ ] Build in-memory adjacency map (file → file, fn → fn)
- [ ] Persist as sqlite edge list
- [ ] Implement queries: `file`, `fn`, `impact` (BFS traversal)
- [ ] JSON output mode
- [ ] Test on a real project — express, rails, or similar

**Success criteria:** `scope impact <file>` correctly lists all transitive dependents.

---

### Phase 2 — Rust Core (3–4 days)

- [ ] Set up workspace with 4 crates
- [ ] `crates/parser`:
  - tree-sitter per language
  - Extract: import paths, export symbols, fn definitions, call sites
  - Resolve relative imports to absolute paths
- [ ] `crates/graph`:
  - petgraph `DiGraph` — node = file or function, edge = import / call
  - Queries: neighbors, BFS impact, DFS tree, cycle detection
- [ ] `crates/store`:
  - Serialize graph as edge list in rusqlite
  - Track file mtime for incremental updates
- [ ] `crates/core`:
  - `scope index .`
  - `scope file <path>` — imports, importers, exports
  - `scope fn <name>` — definition site + all call sites
  - `scope impact <path>` — full transitive dependent set with risk level
  - `scope tree <path> --depth N` — recursive dependency tree
  - `--json` flag throughout

---

### Phase 3 — Incremental Index (1 day)

- [ ] On `index`: skip files whose mtime is unchanged
- [ ] Re-parse only changed files and patch graph edges accordingly
- [ ] On file deletion: remove all edges from and to that node

---

### Phase 4 — Extra Queries (1 day)

- [ ] `scope unused` — exported symbols never imported anywhere (dead exports)
- [ ] `scope cycles` — detect and list circular dependency chains
- [ ] `scope diff <branch>` — which dependents are affected by the current branch

---

### Phase 5 — Polish (1 day)

- [ ] `--json` structured output for all commands
- [ ] `CLAUDE.md` snippet in README
- [ ] Multi-language integration test suite
- [ ] `cargo install scope-cli`

---

## Integration with Claude Code

Add to any project's `CLAUDE.md`:

```markdown
## Before editing any file:
1. `scope file <path>` — understand imports and exports
2. `scope impact <path>` — know the blast radius before touching anything
3. Only then read the files that actually need changing
```

---

## Timeline

| Phase | Duration |
|---|---|
| 1 — POC | 1 day |
| 2 — Rust core | 3–4 days |
| 3 — Incremental index | 1 day |
| 4 — Extra queries | 1 day |
| 5 — Polish | 1 day |
| **Total** | **~1 week** |
