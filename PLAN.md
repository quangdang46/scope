# PLAN: scope — Complete Implementation Reference

## 1. Overview

### Product statement

`scope` is a local static-analysis engine and CLI for coding agents and developers.
It indexes a repository once, persists a dependency graph in SQLite, and answers file,
symbol, and change-impact questions in milliseconds. Pure static analysis, zero LLM,
zero API.

### Core promise

Before editing a file or function, a developer or coding agent can ask:

- What does this file import?
- Who imports this file?
- What symbols does this file expose?
- Which symbols are public vs internal?
- Who calls this function?
- If I rename, delete, or change this, what else is likely affected?
- What is the minimum set of files I need to read to safely complete this task?
- Does my architecture have layer violations?
- Which files are the riskiest to touch right now?
- Why are these two seemingly unrelated files connected?
- Which files secretly co-evolve together even without import edges?
- What would the graph look like if I extracted these symbols into a new module?
- Which entry points can reach this sensitive function?
- How do I safely decompose this god file?
- What is the full public API surface and how has it changed since the last release?
- Which tests cover this file, without running any tests?
- Which files are provably dead code, reachable from no entry point?

### Why this matters

Without an index, coding agents answer dependency questions by opening file after file,
following import chains manually, and exhausting context budget before any real work
begins. `scope` replaces repeated file reads with near-zero-token structured graph queries.

For humans, it surfaces architectural health, refactoring risk, and public API drift that
would otherwise require senior-engineer intuition or expensive tooling to detect. It also
acts as an automated CI guard, preventing architectural degradation on every pull request.

### Positioning

`scope` does **not** try to predict exact runtime behavior. It provides **static
blast-radius analysis** with structured results, clear limitations, and confidence levels
per result node.

---

## 2. Goals

### Primary goals

1. Build a local index of repository dependencies and symbols with zero external services.
2. Answer dependency and impact queries instantly from SQLite — sub-100ms for all standard
   queries on repos up to 10,000 files.
3. Produce structured JSON designed for LLM/tool consumption with stable versioned schemas.
4. Work fully offline with no LLM or API calls required for any feature.
5. Be fast enough to run as a normal part of a development edit loop.
6. Act as an architectural governance tool, not just a query tool.
7. Scale from small personal projects to large monorepos without configuration changes.

### Success criteria

- A developer can answer common dependency questions without opening multiple files.
- A coding agent can ask `scope` for impact data instead of reading 5–20 files directly.
- Query latency feels instant in normal use (sub-100ms warm queries).
- Results are trustworthy because they include reason trails and confidence labels.
- `scope pack` can replace manual context-gathering in agent workflows.
- `scope arch check` and `scope gate` can run in CI and enforce architectural policy.
- `scope report` gives a team a single health score to track over time.
- `scope simulate extract` validates refactoring decisions before touching code.
- `scope serve` makes the graph explorable visually for onboarding and exploration.
- `scope query` REPL enables composable ad-hoc graph exploration.

### Non-goals for v1

- Exact runtime breakage prediction (static analysis only).
- Deep type-system semantic analysis across every language feature.
- Perfect resolution of reflection, dynamic imports, macros, or generated code.
- External package internals or cross-repo analysis.
- IDE-grade automated refactoring or code generation.
- Build system integration (e.g., Bazel, Buck target graphs).
- Binary/bytecode analysis.

---

## 3. Users and jobs-to-be-done

### Primary users

- **Coding agents** (Claude Code and similar) that need structured graph data to avoid
  reading dozens of files speculatively.
- **Individual developers** working in medium and large codebases who need to understand
  dependency topology before making changes.
- **Tech leads and architects** who need automated architectural visibility and enforcement
  without dedicated tooling or expensive subscriptions.
- **Teams** that want CI enforcement of architectural rules and health score tracking.
- **New hires onboarding** to unfamiliar codebases who need to understand structure.

### Jobs to be done

1. **Pre-edit understanding** — "What is this file connected to?" / "Is this symbol public?"
2. **Change planning** — "If I rename this function, what needs to change, and in what order?"
3. **Refactor safety** — "What is the static blast radius of this signature change?"
4. **Agent efficiency** — Replace many file reads with one structured graph query.
5. **Architectural governance** — "Are we accumulating layer violations?" / "Is coupling
   getting worse?"
6. **Release risk** — "What changed in our public API surface since the last release?"
7. **Onboarding** — "Why are these two files connected?" / "What does this module depend on?"
8. **Refactoring simulation** — "What would the graph look like if I extracted these symbols
   into a new module?"
9. **Dead code hunting** — "Which files are provably unreachable from any entry point?"
10. **Security auditing** — "Which entry points can transitively reach network/IO/exec calls?"
11. **Hidden coupling detection** — "Which files secretly co-evolve despite having no import
    edge?"
12. **God-file decomposition** — "How should I split this 900-line utils file?"
13. **Test targeting** — "Which tests should I run after changing this file, without running
    all tests?"
14. **API semver guidance** — "Is this a breaking change, a minor addition, or a patch?"
15. **CI gate enforcement** — "Did this PR introduce any architectural regressions?"

---

## 4. Product boundaries and accuracy model

### Key assumption

The highest-value output is not "the exact truth of runtime behavior"; it is "the best
static graph-based answer with confidence and evidence." Honest uncertainty is more
valuable than false precision.

### Product principle

Every query result should provide:
- Explicit graph evidence (which edges caused this result).
- Transparent limitations (what we could not resolve and why).
- Machine-readable output (JSON with stable schema).
- Stable CLI behavior (no output format changes without schema version bump).

### Certainty levels

`scope` surfaces a certainty label for every edge and impacted node:

- `exact` — directly known from syntax or deterministic resolution. The static analysis
  has unambiguous evidence. Example: `use crate::parser;` in Rust → exact import edge to
  `parser.rs`.
- `resolved` — strongly resolved within repo context, but requires some inference. Example:
  a TypeScript barrel re-export traced through two files.
- `heuristic` — inferred but not fully guaranteed. Example: a dynamic require with a
  partially-known string pattern.
- `dynamic` — known blind spot or unresolved dynamic behavior. Example: `require(varName)`
  with unknown value. The edge exists in output but is labeled as unreliable.

### Conservative resolution principle

v1 should only claim `resolved` when the adapter has sufficient evidence. It is better to
miss a low-confidence edge (false negative) than to invent a false one (false positive).
False positives corrupt impact analysis and erode user trust.

### Failure modes that must be avoided

1. Claiming `exact` certainty for dynamically-dispatched calls.
2. Silently dropping parse errors — all errors must be surfaced in `parse_status`.
3. Crashing on unsupported syntax — always produce partial results.
4. Modifying files without explicit `--apply` flag.
5. Making network requests for any reason.

---

## 5. Architecture

### Why workspace + `crates/` (not `src/`)

- Workspace root holds config, docs, poc, and tests — no source code.
- Each concern lives in its own independently testable crate with its own `Cargo.toml`.
- `scope-core` is a pure library with no binary entry points — can be embedded by other
  tools without pulling in CLI dependencies.
- `scope-cli` is a thin binary that only does argument parsing and output formatting.
- `scope-mcp` is an optional MCP protocol wrapper that can evolve independently.
- Follows the same layout as ripgrep, fd, and similar high-quality Rust CLI tools.
- Workspace-level `Cargo.toml` defines all shared dependencies with pinned versions.

### Crate responsibilities

#### `scope-core` (library)

The heart of the system. Contains all business logic. Has no `main()` and no I/O except
file reading and SQLite. Everything in scope-core must be independently testable without
invoking a subprocess.

Modules:
- `scanner` — file discovery and `.gitignore` handling via the `ignore` crate
- `adapters/` — per-language tree-sitter wrappers with a shared `Adapter` trait
- `extractor` — normalized entity extraction from tree-sitter parse trees
- `resolver` — symbol and import path resolution with certainty annotation
- `store` — SQLite schema definition, migrations, CRUD operations, index_meta
- `graph` — petgraph DiGraph construction, BFS, DFS, Dijkstra, cycle detection,
            SCC analysis, reachability
- `query` — high-level query handlers that compose graph + store operations
- `output` — human-readable formatter and JSON serializer
- `config` — repo root detection, `.scope/` directory management, TOML loading
- `arch` — layer rule parser, pattern matcher, violation detector
- `snapshot` — graph edge-list serialization, snapshot storage and diffing
- `risk` — churn-weighted risk score computation combining git log and graph data
- `stability` — Martin instability metric computation (fan-in/fan-out)
- `test_map` — test file detection and BFS coverage mapping
- `context_pack` — minimum context set scoring and token-budget pack generation
- `rename_plan` — topologically-sorted rename execution plan with byte-offset substitutions
- `surface` — public API surface extraction and diff between refs
- `cochange` — git log co-occurrence matrix computation and temporal coupling analysis
- `simulate` — in-memory graph cloning and hypothetical mutation for what-if analysis
- `entry` — zero-in-degree entry point detection and forward reachability BFS
- `audit` — capability tag loading and reverse-BFS reach analysis
- `split` — symbol caller-pattern clustering for god-file decomposition suggestions
- `mirror` — graph-signature feature vector construction and Jaccard similarity scoring
- `report` — composite health metric aggregation and health score formula
- `gate` — gates.toml parsing and metric threshold evaluation
- `serve` — Axum HTTP server with JSON API endpoints and embedded web UI
- `query_lang` — composable graph query language parser, AST, and evaluator

#### `scope-cli` (binary)

Depends only on `scope-core`. Responsibilities:
- Parse CLI arguments with `clap` derive macros.
- Invoke the appropriate `scope-core` query function.
- Format results using `scope-core`'s output module (human or JSON).
- Handle errors gracefully with human-readable messages.
- Exit with appropriate codes (0 = success, 1 = gate failures/violations, 2 = errors).

No business logic lives in `scope-cli`. It is a thin dispatch layer.

#### `scope-mcp` (optional binary, post-MVP)

Implements the Model Context Protocol stdio interface. Each MCP tool maps to a
`scope-core` query. Produces the same JSON output as `--json` mode. Allows Claude Code
and other MCP-compatible agents to call `scope` as a tool without shell invocation.

---

## 6. File structure

```
scope/
├── Cargo.toml                  # workspace root — no source code here
├── Cargo.lock
├── README.md
├── POC.md
├── PLAN.md
├── CLAUDE.md                   # scope integration instructions for coding agents
├── .gitignore                  # includes .scope/
│
├── .scope/                     # per-repo runtime data (gitignored in user repos)
│   ├── index.db                # SQLite graph database
│   ├── arch.toml               # architectural layer rules and capability tags
│   ├── gates.toml              # CI metric gate thresholds
│   └── snapshots/              # named graph snapshot JSON files
│       └── v1.0.0.json.zst     # compressed snapshot (zstd)
│
├── crates/
│   ├── scope-cli/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs         # clap arg parsing + dispatch to scope-core
│   │
│   ├── scope-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          # public API surface of scope-core
│   │       ├── scanner.rs      # file discovery
│   │       ├── extractor.rs    # tree-sitter extraction → normalized model
│   │       ├── resolver.rs     # import/symbol resolution
│   │       ├── graph.rs        # petgraph DiGraph + traversal algorithms
│   │       ├── store.rs        # SQLite CRUD + migrations
│   │       ├── query.rs        # high-level query composition
│   │       ├── output.rs       # human + JSON formatters
│   │       ├── config.rs       # config loading + repo root detection
│   │       ├── arch.rs         # layer rules + violation detection
│   │       ├── snapshot.rs     # graph snapshot save/load/diff
│   │       ├── risk.rs         # churn-weighted risk scores
│   │       ├── stability.rs    # Martin instability metric
│   │       ├── test_map.rs     # test coverage topology
│   │       ├── context_pack.rs # min context set + token-budget pack
│   │       ├── rename_plan.rs  # topological rename plan + --apply
│   │       ├── surface.rs      # public API surface + diff
│   │       ├── cochange.rs     # git log co-change matrix
│   │       ├── simulate.rs     # in-memory graph mutation
│   │       ├── entry.rs        # entry points + reachability
│   │       ├── audit.rs        # capability reach analysis
│   │       ├── split.rs        # god-file decomposition
│   │       ├── mirror.rs       # graph-signature similarity
│   │       ├── report.rs       # health report generator
│   │       ├── gate.rs         # CI gate evaluation
│   │       ├── serve.rs        # Axum HTTP + embedded web UI
│   │       ├── query_lang.rs   # REPL query language
│   │       └── adapters/
│   │           ├── mod.rs      # Adapter trait + LanguageAdapter enum
│   │           ├── rust.rs     # Rust adapter (tree-sitter-rust)
│   │           ├── javascript.rs  # JS adapter (tree-sitter-javascript)
│   │           └── typescript.rs  # TS adapter (tree-sitter-typescript)
│   │
│   └── scope-mcp/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs         # MCP stdio server (post-MVP)
│
├── poc/
│   ├── package.json
│   └── index.js                # Node.js POC validating extraction accuracy
│
├── fixtures/
│   ├── rust_small/             # ~10 files, straightforward imports, pub/private fns
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── lib.rs
│   │   │   ├── parser.rs
│   │   │   ├── resolver.rs
│   │   │   └── utils.rs
│   │   └── Cargo.toml
│   │
│   ├── rust_medium/            # ~30 files, nested modules, re-exports, trait impls
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── lib.rs
│   │   │   ├── core/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── parser.rs
│   │   │   │   └── types.rs
│   │   │   ├── adapters/
│   │   │   │   ├── mod.rs
│   │   │   │   └── json.rs
│   │   │   └── utils/
│   │   │       ├── mod.rs
│   │   │       └── strings.rs
│   │   └── Cargo.toml
│   │
│   ├── ts_small/               # ~10 files, basic TS imports/exports, barrel files
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   ├── auth/
│   │   │   │   ├── index.ts   # barrel re-export
│   │   │   │   └── middleware.ts
│   │   │   └── utils/
│   │   │       └── logger.ts
│   │   ├── tsconfig.json
│   │   └── package.json
│   │
│   ├── dynamic_limits/         # dynamic requires/imports, should produce 'dynamic' certainty
│   │   ├── src/
│   │   │   ├── dynamic_require.js
│   │   │   ├── computed_import.ts
│   │   │   └── index.js
│   │   └── package.json
│   │
│   ├── arch_violations/        # deliberate layer violations for arch testing
│   │   ├── src/
│   │   │   ├── routes/
│   │   │   │   └── api.js
│   │   │   ├── services/
│   │   │   │   └── user.js
│   │   │   └── utils/
│   │   │       └── token.js   # VIOLATION: imports from services/
│   │   ├── .scope/
│   │   │   └── arch.toml      # rules defining the violation
│   │   └── package.json
│   │
│   ├── rename_fixtures/        # known rename sites for rename-plan testing
│   │   ├── src/
│   │   │   ├── auth/
│   │   │   │   └── middleware.js  # defines verifyToken
│   │   │   ├── routes/
│   │   │   │   ├── api.js         # calls verifyToken
│   │   │   │   └── admin.js       # calls verifyToken
│   │   │   └── tests/
│   │   │       └── auth.test.js   # imports and calls verifyToken
│   │   └── package.json
│   │
│   ├── god_file/               # oversized file with 3 natural symbol clusters
│   │   ├── src/
│   │   │   ├── utils/
│   │   │   │   └── helpers.js  # 50+ exports in 3 unrelated clusters
│   │   │   ├── routes/
│   │   │   │   ├── checkout.js # uses cluster A symbols only
│   │   │   │   └── admin.js    # uses cluster B symbols only
│   │   │   └── services/
│   │   │       └── email.js    # uses cluster C symbols only
│   │   └── package.json
│   │
│   ├── dead_code/              # known unreachable files for entry unreachable testing
│   │   ├── src/
│   │   │   ├── server.js           # entry point
│   │   │   ├── routes/api.js       # reachable
│   │   │   ├── utils/active.js     # reachable
│   │   │   ├── utils/legacy.js     # UNREACHABLE - not imported by anything
│   │   │   └── helpers/old.js      # UNREACHABLE - not imported by anything
│   │   └── package.json
│   │
│   ├── similarity_pairs/       # structurally similar file pairs for mirror testing
│   │   ├── src/
│   │   │   ├── services/
│   │   │   │   ├── stripe.js   # payment provider A — same structure as paypal.js
│   │   │   │   └── paypal.js   # payment provider B
│   │   │   └── routes/
│   │   │       └── checkout.js # calls both
│   │   └── package.json
│   │
│   ├── capability_audit/       # capability tags + known unexpected reach
│   │   ├── src/
│   │   │   ├── server.js           # entry: should reach network
│   │   │   ├── cli.js              # entry: should NOT reach network
│   │   │   ├── http/client.js      # capability: network
│   │   │   └── utils/format.js     # unexpectedly imports http/client.js
│   │   ├── .scope/
│   │   │   └── arch.toml           # capability definitions
│   │   └── package.json
│   │
│   └── cochange/               # git history fixture for temporal coupling tests
│       ├── src/
│       │   ├── auth.js
│       │   ├── constants.js        # always committed with auth.js (hidden coupling)
│       │   └── unrelated.js
│       ├── create_git_history.sh  # script to set up the git fixture
│       └── package.json
│
├── tests/
│   ├── graph_test.rs           # core graph traversal algorithms
│   ├── arch_test.rs            # layer rule matching and violation detection
│   ├── risk_test.rs            # churn + fan-in risk score computation
│   ├── rename_plan_test.rs     # topological ordering + byte-offset correctness
│   ├── cochange_test.rs        # co-occurrence matrix + rate computation
│   ├── simulate_test.rs        # in-memory graph mutation + stability deltas
│   ├── entry_test.rs           # entry point detection + reachability BFS
│   ├── audit_test.rs           # capability reach + unexpected path detection
│   ├── split_test.rs           # symbol clustering + module suggestion correctness
│   ├── gate_test.rs            # gate evaluation + exit code behavior
│   ├── adapter_rust_test.rs    # Rust language adapter extraction accuracy
│   ├── adapter_ts_test.rs      # TypeScript adapter extraction accuracy
│   ├── incremental_test.rs     # incremental index correctness
│   ├── golden/
│   │   ├── deps_basic.json
│   │   ├── deps_reverse.json
│   │   ├── symbols_rust.json
│   │   ├── impact_signature.json
│   │   ├── impact_rename.json
│   │   ├── impact_delete.json
│   │   ├── why_basic.json
│   │   ├── context_basic.json
│   │   ├── risk_basic.json
│   │   ├── cochange_basic.json
│   │   ├── simulate_extract.json
│   │   ├── entry_unreachable.json
│   │   ├── audit_network.json
│   │   ├── split_godfile.json
│   │   ├── mirror_similar.json
│   │   ├── surface_diff.json
│   │   ├── report_basic.json
│   │   └── gate_violations.json
│   └── integration/
│       ├── full_index_rust.rs
│       ├── full_index_ts.rs
│       └── serve_api.rs
│
└── benches/
    ├── index_bench.rs          # full index timing across fixture repos
    ├── query_bench.rs          # query latency measurements
    └── incremental_bench.rs    # incremental re-index timing
```

---

## 7. Root `Cargo.toml`

```toml
[workspace]
members = [
    "crates/scope-core",
    "crates/scope-cli",
    "crates/scope-mcp",
]
resolver = "2"

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"
authors = ["scope contributors"]
repository = "https://github.com/your-org/scope"

[workspace.dependencies]
# Parsing
tree-sitter = "0.22"
tree-sitter-javascript = "0.21"
tree-sitter-typescript = "0.21"
tree-sitter-rust = "0.21"

# Graph
petgraph = { version = "0.6", features = ["serde-1"] }

# Storage
rusqlite = { version = "0.31", features = ["bundled"] }

# File walking
ignore = "0.4"
walkdir = "2"

# CLI
clap = { version = "4", features = ["derive", "env"] }

# Serialization
serde_json = "1"
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# Error handling
anyhow = "1"
thiserror = "1"

# Parallelism
rayon = "1"

# Hashing
blake3 = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

# File watching (watch mode, post-MVP)
notify = "6"
notify-debouncer-mini = "0.4"

# HTTP server (scope serve)
axum = { version = "0.7", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "fs"] }

# REPL
rustyline = { version = "13", features = ["derive"] }

# Compression (snapshots)
zstd = "0.13"

# Token counting (scope pack)
tiktoken-rs = "0.5"

# Testing helpers
tempfile = "3"
```

---

## 8. Language strategy

### Recommendation

Start with **one language family only**. A broad "any language" claim creates quality
problems early because each language has unique import semantics, module systems, and
resolution rules that require careful, tested adapters. The core engine is
language-agnostic; language-specific extraction is adapter-based and introduced gradually.

Preferred rollout order:
1. **Rust** first — ideal for dogfooding `scope` on itself; well-defined module system.
2. **TypeScript/JavaScript** second — highest demand from agent workflows; CommonJS and
   ES module semantics require two sub-adapters sharing a common JS grammar.
3. **Python** third — `import` and `from ... import` are well-defined; dynamic `__import__`
   is labeled `dynamic`.
4. **Additional languages** only after core adapter model has stabilized over at least
   2 language families.

### The Adapter trait

Every language adapter implements the following Rust trait:

```rust
pub trait Adapter: Send + Sync {
    /// File extensions this adapter handles (e.g., ["rs"])
    fn extensions(&self) -> &[&str];

    /// Language identifier stored in the DB (e.g., "rust")
    fn language_id(&self) -> &str;

    /// Parse a source file and return the normalized extraction result.
    /// Must not panic — return ExtractResult with parse_errors populated on failure.
    fn extract(&self, path: &Path, source: &str) -> ExtractResult;
}

pub struct ExtractResult {
    pub imports:     Vec<ImportRecord>,
    pub exports:     Vec<ExportRecord>,
    pub symbols:     Vec<SymbolRecord>,
    pub call_sites:  Vec<CallSiteRecord>,
    pub parse_errors: Vec<ParseError>,
}
```

### Normalized intermediate model

All adapters produce the same normalized types. The `store` module persists these types
without knowing which language produced them.

```rust
pub struct ImportRecord {
    pub raw_text:    String,          // exact text from source ("use crate::parser")
    pub import_path: ImportPath,      // Relative | External | Unresolved
    pub span:        Span,            // byte offsets in source file
    pub certainty:   Certainty,       // exact | resolved | heuristic | dynamic
}

pub enum ImportPath {
    Relative(PathBuf),  // resolved to a repo-relative path
    External(String),   // external package name (not in repo)
    Unresolved,         // could not resolve
}

pub struct ExportRecord {
    pub name:       String,
    pub kind:       SymbolKind,
    pub visibility: Visibility,
    pub span:       Span,
}

pub struct SymbolRecord {
    pub name:       String,
    pub qualname:   String,   // fully qualified: "crate::module::fn_name"
    pub kind:       SymbolKind,
    pub visibility: Visibility,
    pub exported:   bool,
    pub span:       Span,
}

pub struct CallSiteRecord {
    pub callee_name: String,
    pub callee_qualname: Option<String>,  // resolved if possible
    pub span:        Span,
    pub certainty:   Certainty,
}

pub struct ParseError {
    pub message:  String,
    pub span:     Option<Span>,
    pub severity: ParseErrorSeverity,  // Warning | Error
}

pub struct Span {
    pub start_byte: u32,
    pub end_byte:   u32,
    pub start_line: u32,
    pub end_line:   u32,
}

pub enum SymbolKind {
    Function, Method, Struct, Class, Enum, TypeAlias,
    Module, Namespace, Constant, Static, Interface, Trait,
}

pub enum Visibility {
    Local, Module, Package, Public, Unknown,
}

pub enum Certainty {
    Exact, Resolved, Heuristic, Dynamic,
}
```

### Rust adapter implementation notes

The Rust adapter (`adapters/rust.rs`) uses `tree-sitter-rust` grammar.

**Import extraction (use declarations):**
- `use crate::foo::bar` → Relative, resolve `src/foo/bar.rs` or `src/foo/bar/mod.rs`
- `use super::foo` → Relative, resolve relative to current file's parent
- `use std::collections::HashMap` → External("std")
- `use ::external_crate::thing` → External("external_crate")
- Re-exports (`pub use ...`) → both an import AND an export record

**Symbol extraction:**
- `fn foo()` → Function
- `pub fn foo()` → Function, visibility=Public, exported=true
- `impl Foo { fn method() }` → Method with parent Struct reference
- `struct Foo` / `enum Foo` / `type Foo = ...` → Struct/Enum/TypeAlias
- `mod foo;` → creates an import edge to `foo.rs` / `foo/mod.rs`
- `mod foo { }` → inline module, creates a nested namespace scope
- `const FOO: T = ...` / `static FOO: T = ...` → Constant/Static
- `trait Foo` → Trait

**Call site extraction:**
- Direct function calls: `foo()`, `bar::baz()` → CallSiteRecord
- Method calls: `self.foo()` → Method call site
- Macro calls: labeled `heuristic` (macros can call arbitrary code)
- Trait impl method calls: labeled `heuristic` (dynamic dispatch possible)

**Qualname construction:**
- Root of crate: `crate::`
- Module path built from file path: `src/parser/mod.rs` → `crate::parser`
- Symbol in that module: `crate::parser::parse_file`

### TypeScript/JavaScript adapter implementation notes

The TS/JS adapter uses `tree-sitter-typescript` for `.ts`/`.tsx` files and
`tree-sitter-javascript` for `.js`/`.jsx`. Most extraction logic is shared.

**Import extraction:**
- `import { foo } from './bar'` → Relative, resolve `./bar.ts` / `./bar/index.ts`
- `import * as foo from './bar'` → Relative (namespace import)
- `import foo from './bar'` → Relative (default import)
- `require('./bar')` → Relative if string literal, Dynamic if variable
- `import('./bar')` → Relative if string literal, Dynamic if computed
- `import { foo } from 'lodash'` → External("lodash")

**Barrel file detection:**
A file is a barrel if ≥50% of its statements are re-exports (`export { X } from './Y'`).
Barrel files are tagged in `files.parse_status` and their re-exports are traced when
resolving transitive dependencies.

**Symbol extraction:**
- `function foo()` / `const foo = () =>` → Function
- `export function foo()` / `export const foo = ...` → Function/Constant, exported=true
- `export default function` → Function with name `default`
- `class Foo` → Class
- `export class Foo` → Class, exported=true
- TypeScript: `interface Foo` → Interface; `type Foo = ...` → TypeAlias
- `module.exports = { foo }` (CommonJS) → Export records for each key (heuristic certainty)
- `exports.foo = ...` (CommonJS) → Export record (heuristic certainty)

**Resolution rules for relative imports:**
1. `./foo` → try `./foo.ts`, `./foo.tsx`, `./foo.js`, `./foo/index.ts`, `./foo/index.js`
2. `../foo` → same pattern, one directory up
3. Absolute imports (`@/foo`) → resolve via `tsconfig.json` `paths` if present
4. `tsconfig.json` `baseUrl` paths → resolve accordingly

---

## 9. MVP definition

### Must-have in MVP

1. Repository walk respecting `.gitignore` and common ignore patterns.
2. SQLite-backed persistent index in `.scope/index.db`.
3. File-to-file dependency graph with certainty labels.
4. Reverse dependency queries (who imports this file).
5. Top-level symbol extraction with visibility classification.
6. Direct in-repo call graph for resolvable calls.
7. Impact analysis by file or symbol with change-type awareness (all 6 change types).
8. JSON output with `schema_version`, `certainty`, `reason`, `distance` fields on every
   result node.
9. Incremental re-indexing using blake3 content hashing (not just mtime).
10. `scope doctor` to validate index health and report coverage statistics.

### Should-have for MVP

- Transitive dependency traversal with configurable depth limit.
- Reason trails on every impacted node.
- Fixture-based golden JSON tests covering all core commands.
- `scope benchmark` command with full timing breakdown.
- `scope why <a> <b>` — shortest dependency path explanation.
- `scope context <task>` — minimum context set for a task.
- `scope pack` — token-budget-aware context pack for LLM injection.

### Nice-to-have after MVP

- Watch mode (`scope index --watch`).
- MCP server wrapper (`scope-mcp`).
- `scope diff <branch>` — dependents affected by a git branch.
- `scope unused` — dead exports (symbols exported but never imported anywhere).
- `scope cycles` — circular dependency chains with severity scoring.
- `scope tree <path> --depth N` — recursive dependency tree rendering.
- `scope arch check` — layer violation detection with `.scope/arch.toml`.
- `scope risk` — churn-weighted blast radius score using git history.
- `scope stability` — Martin instability metric per file.
- `scope surface` + `scope surface diff` — public API surface and diff.
- `scope rename-plan` — safe topological rename execution plan.
- `scope test-map` — static test coverage topology without instrumentation.
- `scope snapshot` + `scope diff-snapshot` — architectural time travel.
- `scope cochange` — temporal coupling detection via git log co-occurrence.
- `scope simulate extract` — refactoring simulation without touching files.
- `scope entry` — entry point detection, reachability cones, dead code islands.
- `scope audit` — transitive capability reach for security/compliance.
- `scope split` — god-file decomposition suggestions via symbol clustering.
- `scope mirror` — structural similarity detection between files.
- `scope report` — composite codebase health dashboard.
- `scope gate` — metric-based CI enforcement with configurable thresholds.
- `scope serve` — local HTTP server + interactive web UI.
- `scope query` — composable graph query REPL.
- Graph export to DOT/Graphviz and Mermaid diagram format.
- IDE integration via LSP or language server.
- Framework-specific adapters (Express route analysis, Rails concerns, etc.).
- Python, Ruby, Go language adapters.

---

## 10. CLI contract

### Principles

1. **Predictable commands** — users can guess the right command without reading docs.
2. **Every command supports `--json`** — all output is machine-parseable on demand.
3. **Concise human-readable defaults** — no JSON noise in normal terminal use.
4. **Explicit over implicit** — depth limits, change types, and budgets are always flags,
   never silently assumed.
5. **Fail clearly** — error messages explain what went wrong and how to fix it.
6. **Exit codes are meaningful** — 0=success, 1=violations/gate failures, 2=errors.

### Complete command reference

```bash
# ─────────────────────────────────────────────
# INDEXING
# ─────────────────────────────────────────────
scope index                             # full index of CWD repo
scope index --repo-root <path>          # index a specific repo
scope index --no-git                    # skip git log churn population
scope index --watch                     # incremental watch mode (post-MVP)

# ─────────────────────────────────────────────
# FILE GRAPH
# ─────────────────────────────────────────────
scope deps <file>                       # what this file imports (direct)
scope deps <file> --reverse             # what imports this file (direct)
scope deps <file> --transitive          # full transitive import closure
scope deps <file> --transitive --depth 3  # transitive, max depth 3

# ─────────────────────────────────────────────
# SYMBOL INVENTORY
# ─────────────────────────────────────────────
scope symbols <file>                    # all symbols defined in this file
scope symbols <file> --public-only      # only exported/public symbols
scope symbols <file> --kind function    # filter by symbol kind

# ─────────────────────────────────────────────
# CALL GRAPH
# ─────────────────────────────────────────────
scope calls <symbol>                    # what this symbol calls (direct)
scope callers <symbol>                  # what calls this symbol (direct)
scope calls <symbol> --transitive       # transitive callees
scope callers <symbol> --transitive     # transitive callers

# ─────────────────────────────────────────────
# IMPACT ANALYSIS
# ─────────────────────────────────────────────
scope impact <target> --change-type body
scope impact <target> --change-type signature
scope impact <target> --change-type rename
scope impact <target> --change-type delete
scope impact <target> --change-type visibility
scope impact <target> --change-type side-effect
scope impact <target> --change-type signature --depth 3

# ─────────────────────────────────────────────
# DEPENDENCY PATH
# ─────────────────────────────────────────────
scope why <file-a> <file-b>             # shortest path between two nodes
scope why <file-a> <file-b> --all-paths # all paths up to --depth hops
scope explain <target>                  # full evidence trail for a node

# ─────────────────────────────────────────────
# AGENT CONTEXT
# ─────────────────────────────────────────────
scope context <task-description>                         # minimum read set
scope context --target <file> --change-type rename       # explicit target
scope context --target <file> --budget 8000              # with token budget
scope pack <target> --budget 4000                        # formatted context pack
scope pack <target> --budget 2000 --change-type rename   # change-type scoped pack

# ─────────────────────────────────────────────
# REFACTORING
# ─────────────────────────────────────────────
scope rename-plan <old> <new>                # dry-run execution plan
scope rename-plan <old> <new> --apply        # execute the plan
scope rename-plan <old> <new> --apply --force  # include heuristic/dynamic sites

# ─────────────────────────────────────────────
# PUBLIC API SURFACE
# ─────────────────────────────────────────────
scope surface                               # current public surface
scope surface --path src/auth/              # surface for a subtree
scope surface diff HEAD~1 HEAD              # diff since last commit
scope surface diff main feature/auth        # diff between branches
scope surface diff v1.2.0 v1.3.0            # diff between snapshot refs

# ─────────────────────────────────────────────
# ARCHITECTURAL ANALYSIS
# ─────────────────────────────────────────────
scope arch check                            # detect layer violations
scope arch check --strict                   # warnings become errors
scope arch explain <file>                   # which rules apply to this file?
scope arch init                             # generate starter arch.toml
scope stability                             # Martin instability per file
scope stability --file <path>              # single file breakdown
scope stability --flag-threshold 0.5       # only flag files above threshold
scope risk                                  # churn-weighted risk scores
scope risk --days 30                        # 30-day git window
scope risk --file <path>                   # single file breakdown
scope risk --threshold 50                   # only files above risk score

# ─────────────────────────────────────────────
# TEST COVERAGE TOPOLOGY
# ─────────────────────────────────────────────
scope test-map build                        # build the test coverage map
scope test-map covers <source-file>         # which tests cover this file?
scope test-map covered-by <test-file>       # what does this test cover?
scope test-map uncovered                    # source files with no test in cone

# ─────────────────────────────────────────────
# SNAPSHOTS
# ─────────────────────────────────────────────
scope snapshot save --name v1.2.0           # save current graph as snapshot
scope snapshot save --name v1.2.0 --commit HEAD  # tag with git ref
scope snapshot list                          # list saved snapshots
scope snapshot delete v1.0.0                 # delete old snapshot
scope diff-snapshot v1.2.0 v1.3.0            # architectural diff
scope diff-snapshot v1.2.0 v1.3.0 --json

# ─────────────────────────────────────────────
# UTILITIES
# ─────────────────────────────────────────────
scope tree <path> --depth N                 # recursive dependency tree
scope unused                                # dead exports
scope cycles                                # circular dependency chains
scope cycles --severity high                # only high-severity cycles
scope diff <branch>                         # blast radius of branch diff
scope doctor                                # validate index health + coverage
scope benchmark                             # timing breakdown
scope benchmark --repo-root <path> --json

# ─────────────────────────────────────────────
# TEMPORAL COUPLING
# ─────────────────────────────────────────────
scope cochange                              # all unexpected co-change pairs
scope cochange --file <path>               # co-change partners for one file
scope cochange --compare-static             # only pairs with no import edge
scope cochange --threshold 0.7              # pairs co-changing 70%+ of time
scope cochange --days 30                    # shorter git window

# ─────────────────────────────────────────────
# REFACTORING SIMULATION
# ─────────────────────────────────────────────
scope simulate extract <sym,sym,...> --into <new-file>
scope simulate extract <sym,sym,...> --into <new-file> --json

# ─────────────────────────────────────────────
# ENTRY POINTS AND REACHABILITY
# ─────────────────────────────────────────────
scope entry list                            # all entry points
scope entry cone <file>                     # everything reachable from entry
scope entry reaches <file>                  # which entries reach this file?
scope entry unreachable                     # files reachable from no entry

# ─────────────────────────────────────────────
# CAPABILITY / SECURITY AUDIT
# ─────────────────────────────────────────────
scope audit --capability network            # which entries reach network?
scope audit --capability db-write           # which entries reach DB writes?
scope audit --surface                       # public symbols exposing capabilities

# ─────────────────────────────────────────────
# GOD-FILE DECOMPOSITION
# ─────────────────────────────────────────────
scope split <file>                          # suggest natural decomposition
scope split <file> --n 3                    # suggest split into 3 modules

# ─────────────────────────────────────────────
# STRUCTURAL SIMILARITY
# ─────────────────────────────────────────────
scope mirror <file>                         # find structurally similar files
scope mirror <file-a> <file-b>              # compare two files specifically
scope mirror --threshold 0.8                # only pairs above 80% similarity
scope mirror --all                          # scan all pairs (slow on large repos)

# ─────────────────────────────────────────────
# HEALTH REPORTING
# ─────────────────────────────────────────────
scope report                                # full health report to stdout
scope report --output health.md             # save as markdown file
scope report --json                         # machine-readable summary
scope report --compare v1.2.0               # trend report vs snapshot

# ─────────────────────────────────────────────
# CI METRIC GATES
# ─────────────────────────────────────────────
scope gate                                  # evaluate all gates
scope gate --compare main                   # compare vs base branch snapshot
scope gate --strict                         # warnings as errors
scope gate --json                           # machine-readable gate results

# ─────────────────────────────────────────────
# LOCAL WEB UI
# ─────────────────────────────────────────────
scope serve                                 # localhost:7777
scope serve --port 8080                     # custom port
scope serve --open                          # open browser automatically
scope serve --no-ui                         # API only, no web UI

# ─────────────────────────────────────────────
# INTERACTIVE GRAPH REPL
# ─────────────────────────────────────────────
scope query                                 # interactive REPL
scope query --expr 'file "src/auth.rs" | .deps'   # single expression
```

### Important flags reference

```bash
# Universal
--json                      # JSON output for all commands
--repo-root <path>          # override repo root detection
--db <path>                 # override .scope/index.db location
--no-index-check            # skip stale index warning

# Traversal control
--depth <n>                 # max BFS/DFS depth (default: unlimited)
--transitive                # enable transitive traversal
--change-type <type>        # body|signature|rename|delete|visibility|side-effect

# Language/filtering
--language <lang>           # rust|ts|js — filter by language
--public-only               # only public/exported symbols
--kind <kind>               # function|method|struct|class|enum|constant|module

# Git integration
--days <n>                  # git history window in days (default: 90)
--no-git                    # disable all git log integration
--commit <ref>              # specify git ref for snapshot

# Agent/LLM
--budget <tokens>           # token budget for pack/context commands
--output <file>             # write output to file instead of stdout

# Thresholds
--threshold <n>             # numeric threshold for risk/cochange/mirror
--n <count>                 # target number of modules for split

# Simulation/refactoring
--into <file>               # target file for simulate extract
--apply                     # execute rename-plan (default: dry run)
--force                     # include heuristic/dynamic sites in apply

# Arch/gates
--strict                    # treat warnings as errors
--compare <ref>             # compare vs snapshot/branch for gate/report

# Serve/REPL
--port <n>                  # HTTP port for serve (default: 7777)
--open                      # auto-open browser
--no-ui                     # disable web UI, API only
--expr '<query>'            # non-interactive query expression

# Audit
--capability <name>         # capability name for audit command

# Verbosity
--verbose                   # include dynamic/heuristic edges in default output
--quiet                     # suppress progress indicators
```

### Exit codes

```
0  — success, no violations
1  — gate failure (scope gate), arch violations (scope arch check --strict),
     or rename-plan --apply completed with uncertain sites skipped
2  — fatal error (index not found, parse failure, invalid arguments)
```


---

## 11. JSON contract

### Design principles

- Every response includes `schema_version: 1` — bump only on breaking changes.
- Every result node that represents an impact or traversal carries `reason`, `certainty`,
  and `distance`.
- Arrays are always present even when empty — never `null` for collections.
- Unknown future fields are ignored by consumers (forward compatibility).
- File paths are always repo-root-relative with forward slashes, even on Windows.

### `scope index --json`

```json
{
  "schema_version": 1,
  "files_scanned": 847,
  "files_indexed": 23,
  "files_skipped_unchanged": 824,
  "parse_errors": 2,
  "duration_ms": 1847,
  "git_churn_populated": true,
  "index_db": ".scope/index.db"
}
```

### `scope deps <file> --json`

```json
{
  "schema_version": 1,
  "target": "src/resolver.rs",
  "target_kind": "file",
  "direction": "forward",
  "transitive": false,
  "dependencies": [
    {
      "kind": "file",
      "path": "src/parser.rs",
      "edge_kind": "import",
      "certainty": "exact",
      "import_text": "use crate::parser",
      "line": 3
    },
    {
      "kind": "file",
      "path": "src/types.rs",
      "edge_kind": "import",
      "certainty": "exact",
      "import_text": "use crate::types::*",
      "line": 4
    }
  ],
  "total": 2
}
```

### `scope deps <file> --reverse --json`

```json
{
  "schema_version": 1,
  "target": "src/resolver.rs",
  "target_kind": "file",
  "direction": "reverse",
  "transitive": false,
  "dependencies": [
    {
      "kind": "file",
      "path": "src/cli.rs",
      "edge_kind": "import",
      "certainty": "exact"
    },
    {
      "kind": "file",
      "path": "src/index/mod.rs",
      "edge_kind": "import",
      "certainty": "exact"
    }
  ],
  "total": 2
}
```

### `scope symbols <file> --json`

```json
{
  "schema_version": 1,
  "file": "src/resolver.rs",
  "language": "rust",
  "symbols": [
    {
      "qualname": "crate::resolver::resolve_symbol",
      "name": "resolve_symbol",
      "kind": "function",
      "visibility": "public",
      "exported": true,
      "span": {
        "start_byte": 120,
        "end_byte": 580,
        "start_line": 14,
        "end_line": 34
      }
    },
    {
      "qualname": "crate::resolver::ImportResolver",
      "name": "ImportResolver",
      "kind": "struct",
      "visibility": "public",
      "exported": true,
      "span": {
        "start_byte": 600,
        "end_byte": 720,
        "start_line": 37,
        "end_line": 44
      }
    },
    {
      "qualname": "crate::resolver::resolve_internal",
      "name": "resolve_internal",
      "kind": "function",
      "visibility": "module",
      "exported": false,
      "span": {
        "start_byte": 800,
        "end_byte": 1100,
        "start_line": 47,
        "end_line": 68
      }
    }
  ],
  "total": 3
}
```

### `scope calls <symbol> --json`

```json
{
  "schema_version": 1,
  "target": "crate::resolver::resolve_symbol",
  "target_kind": "function",
  "direction": "callees",
  "call_sites": [
    {
      "callee": "crate::parser::parse_import",
      "callee_file": "src/parser.rs",
      "caller_file": "src/resolver.rs",
      "line": 22,
      "certainty": "exact"
    },
    {
      "callee": "crate::types::ImportPath::new",
      "callee_file": "src/types.rs",
      "caller_file": "src/resolver.rs",
      "line": 28,
      "certainty": "resolved"
    }
  ],
  "total": 2
}
```

### `scope impact <target> --change-type signature --json`

```json
{
  "schema_version": 1,
  "target": "crate::resolver::resolve_symbol",
  "target_kind": "function",
  "change_type": "signature",
  "affected": [
    {
      "kind": "function",
      "qualname": "crate::impact::compute_impact",
      "name": "compute_impact",
      "file": "src/impact.rs",
      "reason": "calls crate::resolver::resolve_symbol directly",
      "edge_kind": "call",
      "distance": 1,
      "certainty": "resolved"
    },
    {
      "kind": "function",
      "qualname": "crate::cli::run_query",
      "name": "run_query",
      "file": "src/cli.rs",
      "reason": "calls crate::impact::compute_impact (which calls target)",
      "edge_kind": "call",
      "distance": 2,
      "certainty": "resolved"
    }
  ],
  "uncertain": [
    {
      "kind": "function",
      "qualname": "crate::mcp::handle_request",
      "name": "handle_request",
      "file": "src/mcp.rs",
      "reason": "dynamic dispatch path includes target module",
      "edge_kind": "dynamic",
      "distance": 3,
      "certainty": "dynamic"
    }
  ],
  "summary": {
    "high_confidence_count": 2,
    "uncertain_count": 1,
    "max_distance": 3
  }
}
```

### `scope why <file-a> <file-b> --json`

```json
{
  "schema_version": 1,
  "from": "src/utils/logger.rs",
  "to": "src/routes/payments.rs",
  "paths": [
    {
      "length": 3,
      "certainty": "resolved",
      "hops": [
        {
          "file": "src/utils/logger.rs",
          "role": "source"
        },
        {
          "file": "src/services/stripe.rs",
          "role": "intermediate",
          "edge_kind": "import",
          "line": 4,
          "import_text": "use crate::utils::logger",
          "certainty": "exact"
        },
        {
          "file": "src/routes/payments.rs",
          "role": "destination",
          "edge_kind": "import",
          "line": 2,
          "import_text": "use crate::services::stripe",
          "certainty": "exact"
        }
      ]
    },
    {
      "length": 4,
      "certainty": "resolved",
      "hops": [
        { "file": "src/utils/logger.rs", "role": "source" },
        { "file": "src/middleware/request.rs", "role": "intermediate", "edge_kind": "import", "line": 11 },
        { "file": "src/services/webhook.rs", "role": "intermediate", "edge_kind": "import", "line": 7 },
        { "file": "src/routes/payments.rs", "role": "destination", "edge_kind": "import", "line": 8 }
      ]
    }
  ],
  "total_paths_found": 2
}
```

### `scope context --json`

```json
{
  "schema_version": 1,
  "task": "rename verify_token in auth middleware and update all callers",
  "targets_identified": ["src/auth/middleware.js"],
  "must_read": [
    {
      "file": "src/auth/middleware.js",
      "reason": "defines verify_token — primary change target",
      "priority": 1,
      "estimated_tokens": 340
    },
    {
      "file": "src/routes/api.js",
      "reason": "direct caller of verify_token (2 call sites)",
      "priority": 2,
      "estimated_tokens": 280
    },
    {
      "file": "src/routes/admin.js",
      "reason": "direct caller of verify_token (1 call site)",
      "priority": 3,
      "estimated_tokens": 190
    },
    {
      "file": "src/utils/jwt.js",
      "reason": "direct callee of verify_token — understand what it calls",
      "priority": 4,
      "estimated_tokens": 210
    }
  ],
  "should_read": [
    {
      "file": "src/tests/auth.test.js",
      "reason": "tests verify_token behavior — update after rename",
      "priority": 5,
      "estimated_tokens": 380
    }
  ],
  "skip": [
    {
      "file": "src/routes/health.js",
      "reason": "no graph connection to auth graph"
    },
    {
      "file": "src/config/db.js",
      "reason": "no graph connection to auth graph"
    }
  ],
  "total_must_read_tokens": 1020,
  "total_should_read_tokens": 380
}
```

### `scope pack` output format (plain text, not JSON)

```
=== SCOPE CONTEXT PACK ===
Target:      src/auth/middleware.js
Change type: rename
Budget:      4000 tokens
Used:        1847 tokens
Generated:   2025-03-08T14:22:00Z
Schema:      1

─── PUBLIC SURFACE (src/auth/middleware.js) ─────────────────
Exports:
  verifyToken(req, res, next)   function   public   line 12
  requireAdmin(req, res, next)  function   public   line 34

─── DIRECT CALLERS (certainty: exact) ───────────────────────
src/routes/api.js:14       router.use('/api', verifyToken)
src/routes/api.js:3        import { verifyToken } from '../auth/middleware'
src/routes/admin.js:8      router.use(verifyToken, requireAdmin)
src/routes/admin.js:2      import { verifyToken } from '../auth/middleware'

─── DIRECT CALLEES (what verifyToken calls) ─────────────────
src/utils/jwt.js           jwt.verify() — certainty: exact, line 18
src/models/user.js         User.findById() — certainty: resolved, line 22

─── TRANSITIVE CALLERS (distance ≤ 2) ───────────────────────
src/app.js                 mounts /api (imports routes/api.js) — distance: 2

─── RENAME IMPACT SUMMARY ───────────────────────────────────
  3 import sites to update
  2 call sites to update
  0 re-exports to update
  1 test file to update (src/tests/auth.test.js)

=== END PACK ===
```

### `scope risk --json`

```json
{
  "schema_version": 1,
  "window_days": 90,
  "computed_at": "2025-03-08T14:22:00Z",
  "files": [
    {
      "path": "src/auth/token.js",
      "risk_score": 94,
      "risk_level": "critical",
      "transitive_dependents": 47,
      "direct_dependents": 12,
      "commits_in_window": 23,
      "formula": "log(1 + 47) * log(1 + 23)"
    },
    {
      "path": "src/db/connection.js",
      "risk_score": 71,
      "risk_level": "high",
      "transitive_dependents": 38,
      "direct_dependents": 8,
      "commits_in_window": 8,
      "formula": "log(1 + 38) * log(1 + 8)"
    }
  ],
  "total": 847,
  "critical": 4,
  "high": 7,
  "medium": 23,
  "low": 813
}
```

### `scope stability --json`

```json
{
  "schema_version": 1,
  "files": [
    {
      "path": "src/utils/constants.js",
      "instability": 0.02,
      "fan_in": 48,
      "fan_out": 1,
      "category": "stable",
      "flagged": false
    },
    {
      "path": "src/db/connection.js",
      "instability": 0.71,
      "fan_in": 29,
      "fan_out": 71,
      "category": "unstable_and_central",
      "flagged": true,
      "reason": "high fan-in (29) but also high instability — structural liability"
    }
  ],
  "summary": {
    "avg_instability": 0.41,
    "flagged_count": 3,
    "stable_count": 12,
    "healthy_leaf_count": 198
  }
}
```

### `scope surface diff --json`

```json
{
  "schema_version": 1,
  "from_ref": "v1.2.0",
  "to_ref": "v1.3.0",
  "removed": [
    {
      "qualname": "crate::auth::verify_token",
      "kind": "function",
      "visibility": "public",
      "file": "src/auth/mod.rs",
      "signature_before": "verify_token(token: &str) -> bool"
    }
  ],
  "added": [
    {
      "qualname": "crate::auth::VerifyOpts",
      "kind": "struct",
      "visibility": "public",
      "file": "src/auth/mod.rs"
    },
    {
      "qualname": "crate::auth::verify_token_with_opts",
      "kind": "function",
      "visibility": "public",
      "file": "src/auth/mod.rs",
      "signature_after": "verify_token_with_opts(token: &str, opts: VerifyOpts) -> Result<Claims>"
    }
  ],
  "changed": [],
  "semver_recommendation": "minor",
  "semver_reason": "added 2 symbols, removed 1 — removal makes this at least minor; if callers exist outside repo it may be major",
  "summary": {
    "removed_count": 1,
    "added_count": 2,
    "changed_count": 0,
    "total_public_symbols": 314
  }
}
```

### `scope cochange --json`

```json
{
  "schema_version": 1,
  "file": "src/auth/middleware.js",
  "window_days": 90,
  "commits_analyzed": 847,
  "co_changes": [
    {
      "partner": "src/config/constants.js",
      "co_change_rate": 0.89,
      "shared_commits": 34,
      "total_commits_file": 38,
      "total_commits_partner": 36,
      "has_static_edge": false,
      "classification": "unexpected",
      "suggestion": "consider adding an explicit import or extracting the shared concern"
    },
    {
      "partner": "src/tests/auth.test.js",
      "co_change_rate": 0.91,
      "shared_commits": 35,
      "total_commits_file": 38,
      "total_commits_partner": 37,
      "has_static_edge": true,
      "classification": "expected"
    }
  ],
  "unexpected_count": 1,
  "expected_count": 1
}
```

### `scope simulate extract --json`

```json
{
  "schema_version": 1,
  "extraction": {
    "symbols": ["verifyToken", "createSession", "destroySession"],
    "from_file": "src/auth/middleware.js",
    "into_file": "src/auth/session.js"
  },
  "graph_delta": {
    "edges_added": 4,
    "edges_removed": 4,
    "new_edges": [
      { "from": "src/routes/api.js",      "to": "src/auth/session.js",   "kind": "import" },
      { "from": "src/routes/admin.js",    "to": "src/auth/session.js",   "kind": "import" },
      { "from": "src/auth/middleware.js", "to": "src/auth/session.js",   "kind": "import" },
      { "from": "src/tests/auth.test.js", "to": "src/auth/session.js",   "kind": "import" }
    ],
    "removed_edges": [
      { "from": "src/routes/api.js",      "to": "src/auth/middleware.js", "kind": "import" },
      { "from": "src/routes/admin.js",    "to": "src/auth/middleware.js", "kind": "import" },
      { "from": "src/tests/auth.test.js", "to": "src/auth/middleware.js", "kind": "import" }
    ],
    "cycles_introduced": 0,
    "cycles_resolved": 1,
    "new_layer_violations": 0,
    "resolved_layer_violations": 0
  },
  "stability_delta": [
    {
      "file": "src/auth/middleware.js",
      "instability_before": 0.71,
      "instability_after": 0.43,
      "improved": true
    },
    {
      "file": "src/auth/session.js",
      "instability_before": null,
      "instability_after": 0.18,
      "improved": true,
      "note": "new file — stable (many callers, few deps)"
    }
  ],
  "recommendation": "high_value",
  "recommendation_reasons": [
    "reduces instability of source file",
    "resolves 1 existing cycle",
    "creates a clean stable abstraction (I=0.18)"
  ]
}
```

### `scope entry unreachable --json`

```json
{
  "schema_version": 1,
  "entry_points": [
    { "file": "src/server.js", "detection": "zero_in_degree" },
    { "file": "src/cli.js",    "detection": "zero_in_degree" }
  ],
  "total_files": 847,
  "reachable_files": 844,
  "unreachable_files": 3,
  "unreachable": [
    {
      "file": "src/utils/legacy_crypto.js",
      "last_modified_days_ago": 847,
      "exported_symbols": 4,
      "certainty": "heuristic",
      "certainty_note": "dynamic imports may create edges not modeled statically"
    },
    {
      "file": "src/helpers/xml_parser.js",
      "last_modified_days_ago": 203,
      "exported_symbols": 7,
      "certainty": "heuristic"
    },
    {
      "file": "src/types/deprecated.ts",
      "last_modified_days_ago": 1203,
      "exported_symbols": 12,
      "certainty": "exact",
      "certainty_note": "TypeScript — dynamic imports fully detected"
    }
  ]
}
```

### `scope audit --capability network --json`

```json
{
  "schema_version": 1,
  "capability": "network",
  "capability_sources": [
    "src/http/client.js",
    "src/http/webhook.js",
    "src/integrations/stripe.js",
    "src/integrations/sendgrid.js"
  ],
  "entry_points_with_reach": [
    {
      "file": "src/server.js",
      "expected": true,
      "shortest_path_length": 3
    },
    {
      "file": "src/workers/sync.js",
      "expected": true,
      "shortest_path_length": 2
    },
    {
      "file": "src/cli.js",
      "expected": false,
      "unexpected": true,
      "shortest_path_length": 4
    },
    {
      "file": "src/scripts/migrate.js",
      "expected": false,
      "unexpected": true,
      "shortest_path_length": 3
    }
  ],
  "unexpected_paths": [
    {
      "entry": "src/cli.js",
      "path": [
        "src/cli.js",
        "src/utils/format.js",
        "src/integrations/stripe.js"
      ],
      "certainty": "resolved",
      "note": "format utility imports stripe for currency formatting — likely unintentional"
    },
    {
      "entry": "src/scripts/migrate.js",
      "path": [
        "src/scripts/migrate.js",
        "src/services/email.js",
        "src/integrations/sendgrid.js"
      ],
      "certainty": "resolved",
      "note": "migration script triggers welcome emails — likely a bug"
    }
  ],
  "summary": {
    "expected_entry_points": 2,
    "unexpected_entry_points": 2
  }
}
```

### `scope gate --json`

```json
{
  "schema_version": 1,
  "passed": false,
  "compared_to": "main",
  "gates": [
    {
      "metric": "layer_violations",
      "current_value": 0,
      "max": 0,
      "passed": true,
      "severity": "error"
    },
    {
      "metric": "cycles",
      "current_value": 0,
      "max": 0,
      "passed": true,
      "severity": "error"
    },
    {
      "metric": "max_file_fan_in",
      "current_value": 54,
      "max": 50,
      "passed": false,
      "severity": "warning",
      "violating_file": "src/auth/middleware.js",
      "message": "A file has too many dependents — consider splitting"
    },
    {
      "metric": "health_score_delta",
      "current_value": 3,
      "min_delta": -5,
      "passed": true,
      "severity": "error"
    },
    {
      "metric": "public_surface_removed",
      "current_value": 2,
      "max": 0,
      "passed": false,
      "severity": "error",
      "detail": [
        "crate::auth::verify_token",
        "crate::auth::create_session"
      ],
      "message": "Public API symbols removed — this is a breaking change"
    }
  ],
  "errors": 1,
  "warnings": 1,
  "exit_code": 1
}
```

### `scope report --json`

```json
{
  "schema_version": 1,
  "generated_at": "2025-03-08T14:22:00Z",
  "health_score": 73,
  "health_score_delta": 4,
  "compared_to": "v1.2.0",
  "sections": {
    "coverage": {
      "imports_resolved_pct": 94.2,
      "imports_total": 3847,
      "imports_resolved": 3625,
      "parse_errors": 3,
      "parse_error_files": ["src/generated/proto.js"],
      "languages": { "rust": 421, "typescript": 314, "javascript": 112 }
    },
    "size": {
      "total_files": 847,
      "total_symbols": 12341,
      "avg_fan_in": 3.2,
      "max_fan_in": 47,
      "max_fan_in_file": "src/auth/token.js",
      "avg_fan_out": 3.8,
      "max_fan_out": 71
    },
    "risk": {
      "critical_files": 4,
      "high_files": 7,
      "medium_files": 23,
      "top_risk_file": "src/auth/token.js",
      "top_risk_score": 94
    },
    "arch": {
      "layer_violations": 2,
      "cycles": 1,
      "avg_instability": 0.41,
      "flagged_unstable_central": 3
    },
    "dead_code": {
      "unreachable_files": 3,
      "unused_exports": 18,
      "uncovered_source_files": 47
    },
    "temporal_coupling": {
      "unexpected_cochange_pairs": 7,
      "god_file_candidates": 3
    },
    "surface": {
      "total_public_symbols": 312,
      "added_since_snapshot": 8,
      "removed_since_snapshot": 2,
      "semver_delta": "minor"
    }
  },
  "health_score_breakdown": {
    "base": 100,
    "layer_violations_penalty": -10,
    "cycles_penalty": -10,
    "unreachable_files_penalty": -6,
    "unexpected_cochange_penalty": -21,
    "god_files_penalty": -12,
    "coverage_bonus": 18.84,
    "total": 73
  }
}
```

### `scope diff-snapshot v1.2.0 v1.3.0 --json`

```json
{
  "schema_version": 1,
  "from_snapshot": "v1.2.0",
  "to_snapshot": "v1.3.0",
  "created_at": "2025-03-08T14:22:00Z",
  "edge_changes": {
    "added": 49,
    "removed": 12,
    "net": 37
  },
  "new_central_dependencies": [
    {
      "file": "src/services/fraud.js",
      "fan_in_before": 0,
      "fan_in_after": 14,
      "note": "newly depended-upon by many files — monitor stability"
    }
  ],
  "cycles": {
    "before": 1,
    "after": 1,
    "introduced": 0,
    "resolved": 0
  },
  "layer_violations": {
    "before": 0,
    "after": 1,
    "introduced": 1,
    "resolved": 0,
    "new_violations": [
      {
        "from": "src/utils/fraud_helpers.js",
        "to": "src/services/payment.js",
        "rule": "utils must not import services"
      }
    ]
  },
  "stability": {
    "avg_instability_before": 0.39,
    "avg_instability_after": 0.41,
    "delta": 0.02,
    "direction": "worse"
  },
  "surface": {
    "removed_count": 1,
    "added_count": 2,
    "semver_recommendation": "minor"
  }
}
```

---

## 12. Change-impact model

### Why change type matters

Impact is only meaningful if the tool knows **what kind of change** is being made.
A function body change has a different blast radius from a rename or signature change.
The same file may have zero dependents that need updating for a `body` change, but 40
dependents that need updating for a `rename` change.

### Supported change types

| Change type   | Meaning                                              | Traversal strategy | Primary blast radius |
|---------------|------------------------------------------------------|-------------------|----------------------|
| `body`        | Implementation changes; public shape unchanged       | Call graph only   | Direct callers, behavioral tests |
| `signature`   | Parameter list, return type, or contract changes     | Call + import     | All callers, transitive wrappers, exported API consumers |
| `rename`      | Symbol/file/module name changes                      | All edges         | All references, import sites, re-exports, re-export chains |
| `delete`      | Target removed entirely                              | All edges         | All reverse dependencies — callers and importers both fail |
| `visibility`  | Target becomes more or less accessible               | Visibility + import | External consumers outside the visibility boundary, re-export graph |
| `side-effect` | File-level initialization/execution behavior changes | File import only  | All importers, transitive importers (if init order matters) |

### Impact algorithm (detailed)

**Input:** target node (file or symbol), change_type

**Step 1 — Resolve target:**
- If target is a file path: look up `files.path`.
- If target is a qualname: look up `symbols.qualname`.
- If not found: return error with suggestions (did you forget to `scope index`?).

**Step 2 — Select traversal strategy:**

```
change_type = "body":
  Walk: symbol_edges WHERE kind='call', direction=reverse (callers only)
  Stop at: distance=1 (direct callers) unless --transitive
  Certainty filter: include exact, resolved; mark heuristic as uncertain; exclude dynamic
    from default output (include with --verbose)

change_type = "signature":
  Walk: symbol_edges WHERE kind='call', direction=reverse, ALL distances
  Also include: file_edges WHERE kind='import' AND to_file=files_containing_target_symbol
  Include: re-export chains (symbols re-exporting this symbol)
  All distances included

change_type = "rename":
  Walk: ALL symbol_edges referencing this symbol (call + import)
  Walk: ALL file_edges importing this file
  Include: re-export chain (all files re-exporting this symbol)
  Include: string references labeled heuristic

change_type = "delete":
  Same as rename traversal PLUS:
  Mark all results as "reference will break" rather than "needs update"

change_type = "visibility" (narrowing, e.g., pub → pub(crate)):
  Walk: file_edges WHERE from_file is OUTSIDE the new visibility boundary
  Walk: symbol_edges WHERE from_symbol is in a file outside boundary
  Note: visibility broadening (private → pub) has empty blast radius

change_type = "side-effect":
  Walk: file_edges WHERE kind='import', direction=reverse (importers)
  Walk transitive importers up to configurable depth
  All results labeled with distance from source
```

**Step 3 — Annotate results:**
For each reached node:
- `reason` = human-readable string describing which edge caused this node to appear
- `distance` = hop count from target
- `certainty` = minimum certainty of all edges on the path to this node
- `edge_kind` = the type of the edge that caused inclusion (`call`, `import`, `re-export`)

**Step 4 — Group and sort:**
- Group A: `certainty` is `exact` or `resolved` → "Affected (high confidence)"
- Group B: `certainty` is `heuristic` or `dynamic` → "Affected (uncertain)"
- Within each group: sort by `distance` ascending, then `file` alphabetically

---

## 13. Persistence design

### Database location and discovery

`scope` stores its index in `.scope/index.db` relative to the repo root. Repo root
detection:
1. Walk up from CWD looking for `.git/`, `Cargo.toml` (workspace), `package.json`
   (with `workspaces`), or `.scope/`.
2. If not found, use CWD as repo root.
3. Create `.scope/` if it does not exist.
4. User can always override with `--repo-root <path>` or `--db <path>`.

### Migration system

Every schema change increments the `user_version` pragma. On DB open, `scope` checks
`PRAGMA user_version`, and runs migrations forward as needed. Migrations are embedded Rust
const strings, never external files.

```rust
const MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    include_str!("migrations/001_initial.sql"),
    // v2 — add file_cochange table
    include_str!("migrations/002_cochange.sql"),
    // v3 — add capabilities table
    include_str!("migrations/003_capabilities.sql"),
];
```

On first open: run all migrations. On existing DB: run only migrations with index >
current `user_version`.

### Complete schema

```sql
PRAGMA journal_mode = WAL;   -- concurrent reads during queries
PRAGMA foreign_keys = ON;
PRAGMA user_version = 3;     -- incremented on each migration

-- ─────────────────────────────────────────────
-- Core entities
-- ─────────────────────────────────────────────

CREATE TABLE files (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  path         TEXT    NOT NULL UNIQUE,    -- repo-root-relative, forward slashes
  language     TEXT    NOT NULL,           -- "rust" | "typescript" | "javascript"
  hash         TEXT    NOT NULL,           -- blake3 hex string
  mtime        INTEGER,                    -- unix timestamp, secondary check only
  parse_status TEXT    NOT NULL,           -- "ok" | "error" | "partial"
  parse_error  TEXT,                       -- error message if parse_status != "ok"
  indexed_at   INTEGER NOT NULL,           -- unix timestamp of last index
  line_count   INTEGER                     -- for reporting
);

CREATE TABLE symbols (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  file_id    INTEGER NOT NULL,
  qualname   TEXT    NOT NULL,             -- "crate::resolver::resolve_symbol"
  name       TEXT    NOT NULL,             -- "resolve_symbol"
  kind       TEXT    NOT NULL,             -- SymbolKind enum as text
  visibility TEXT    NOT NULL,             -- Visibility enum as text
  exported   INTEGER NOT NULL DEFAULT 0,  -- 1 if exported/pub
  span_start INTEGER,                     -- start byte offset
  span_end   INTEGER,                     -- end byte offset
  start_line INTEGER,                     -- start line number (1-indexed)
  end_line   INTEGER,                     -- end line number
  FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
  UNIQUE(file_id, qualname)
);

CREATE TABLE imports (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  file_id          INTEGER NOT NULL,
  raw_text         TEXT    NOT NULL,       -- exact import text from source
  resolved_file_id INTEGER,               -- NULL if unresolved
  import_path_kind TEXT    NOT NULL,       -- "relative" | "external" | "unresolved"
  external_pkg     TEXT,                  -- package name if import_path_kind="external"
  span_start       INTEGER,
  span_end         INTEGER,
  start_line       INTEGER,
  certainty        TEXT    NOT NULL,       -- Certainty enum
  FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
  FOREIGN KEY(resolved_file_id) REFERENCES files(id) ON DELETE SET NULL
);

CREATE TABLE file_edges (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  from_file_id INTEGER NOT NULL,
  to_file_id   INTEGER NOT NULL,
  kind         TEXT    NOT NULL,           -- "import" | "re-export"
  certainty    TEXT    NOT NULL,
  FOREIGN KEY(from_file_id) REFERENCES files(id) ON DELETE CASCADE,
  FOREIGN KEY(to_file_id)   REFERENCES files(id) ON DELETE CASCADE,
  UNIQUE(from_file_id, to_file_id, kind)
);

CREATE TABLE symbol_edges (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  from_symbol_id INTEGER NOT NULL,
  to_symbol_id   INTEGER NOT NULL,
  kind           TEXT    NOT NULL,         -- "call" | "import" | "re-export" | "impl"
  certainty      TEXT    NOT NULL,
  call_line      INTEGER,                  -- source line of the call site
  FOREIGN KEY(from_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
  FOREIGN KEY(to_symbol_id)   REFERENCES symbols(id) ON DELETE CASCADE
);

-- ─────────────────────────────────────────────
-- Git integration
-- ─────────────────────────────────────────────

CREATE TABLE file_churn (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  file_id    INTEGER NOT NULL,
  commit_sha TEXT    NOT NULL,
  author     TEXT,
  timestamp  INTEGER NOT NULL,             -- unix timestamp of commit
  FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
  UNIQUE(file_id, commit_sha)
);

CREATE TABLE file_cochange (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  file_a_id       INTEGER NOT NULL,
  file_b_id       INTEGER NOT NULL,       -- always file_a_id < file_b_id (canonical order)
  shared_commits  INTEGER NOT NULL,
  total_commits_a INTEGER NOT NULL,
  total_commits_b INTEGER NOT NULL,
  window_days     INTEGER NOT NULL,
  computed_at     INTEGER NOT NULL,       -- unix timestamp
  FOREIGN KEY(file_a_id) REFERENCES files(id) ON DELETE CASCADE,
  FOREIGN KEY(file_b_id) REFERENCES files(id) ON DELETE CASCADE,
  UNIQUE(file_a_id, file_b_id, window_days)
);

-- ─────────────────────────────────────────────
-- Snapshots and metadata
-- ─────────────────────────────────────────────

CREATE TABLE snapshots (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT    NOT NULL UNIQUE,
  git_ref     TEXT,                        -- git commit SHA or tag if provided
  created_at  INTEGER NOT NULL,
  edge_json   BLOB    NOT NULL             -- zstd-compressed JSON edge list
);

CREATE TABLE index_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- Required index_meta keys:
--   "repo_root"         — absolute path to repo root
--   "last_index_at"     — unix timestamp
--   "schema_version"    — "3"
--   "total_files"       — count at last index
--   "total_symbols"     — count at last index
--   "git_available"     — "true" | "false"

-- ─────────────────────────────────────────────
-- User-defined capabilities (from arch.toml)
-- ─────────────────────────────────────────────

CREATE TABLE capabilities (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT    NOT NULL,             -- "network" | "db-write" | "exec" etc.
  match_kind TEXT    NOT NULL,             -- "file_pattern" | "symbol_name" | "file_id"
  pattern    TEXT,                         -- glob pattern for file_pattern
  symbol     TEXT,                         -- symbol name for symbol_name
  file_id    INTEGER,                      -- direct file ref for file_id
  FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
);
```

### Required indices

```sql
-- Core lookups
CREATE INDEX idx_files_path          ON files(path);
CREATE INDEX idx_files_language      ON files(language);
CREATE INDEX idx_symbols_qualname    ON symbols(qualname);
CREATE INDEX idx_symbols_name        ON symbols(name);
CREATE INDEX idx_symbols_file_id     ON symbols(file_id);
CREATE INDEX idx_symbols_kind        ON symbols(kind);
CREATE INDEX idx_symbols_visibility  ON symbols(visibility);
CREATE INDEX idx_symbols_exported    ON symbols(exported);

-- Edge traversal — these are the hot paths
CREATE INDEX idx_file_edges_from     ON file_edges(from_file_id);
CREATE INDEX idx_file_edges_to       ON file_edges(to_file_id);
CREATE INDEX idx_symbol_edges_from   ON symbol_edges(from_symbol_id);
CREATE INDEX idx_symbol_edges_to     ON symbol_edges(to_symbol_id);

-- Import resolution
CREATE INDEX idx_imports_file_id     ON imports(file_id);
CREATE INDEX idx_imports_resolved    ON imports(resolved_file_id);

-- Git integration
CREATE INDEX idx_file_churn_file_id  ON file_churn(file_id);
CREATE INDEX idx_file_churn_ts       ON file_churn(timestamp);
CREATE INDEX idx_file_cochange_a     ON file_cochange(file_a_id);
CREATE INDEX idx_file_cochange_b     ON file_cochange(file_b_id);

-- Capability queries
CREATE INDEX idx_capabilities_name   ON capabilities(name);
```

### Storage rules and invariants

1. All file paths stored repo-root-relative with forward slashes on all platforms.
2. Always store blake3 content hash (`blake3::hash(content).to_hex()`) — not mtime alone.
   Mtime is stored as a secondary optimization hint but never trusted for correctness.
3. `qualname` is the canonical unique identifier for a symbol — it incorporates the full
   module path. Two symbols with the same `name` in different modules have different
   `qualname` values.
4. On re-index of a changed file: DELETE all symbols, imports, edges for that file (ON
   DELETE CASCADE handles children), then re-insert fresh data.
5. On file deletion: same CASCADE DELETE. No orphan edges.
6. `file_edges` rows are deduplicated by `(from_file_id, to_file_id, kind)` — multiple
   import statements importing the same file produce one edge row.
7. `file_cochange` stores only pairs where `file_a_id < file_b_id` to avoid duplicates.
8. `index_meta` is always updated atomically at the end of a successful index run.

---

## 14. Indexing pipeline

### Configuration loading

Before indexing, `scope` loads configuration:

1. Check `--repo-root` flag; otherwise auto-detect by walking up from CWD.
2. Create `.scope/` directory if missing.
3. Open/create `.scope/index.db` and run pending migrations.
4. Load `.scope/arch.toml` if present (capabilities, layers, test file patterns).
5. Load `.scope/gates.toml` if present (only needed for `scope gate`).
6. Determine which language adapters to use based on file extensions in repo
   (or `--language` flag to restrict).

### Full index pipeline (detailed)

```
1. Repo root detection
   └── walk CWD upward looking for: .git/, Cargo.toml (workspace), package.json
       If not found: warn and use CWD

2. DB open + migration
   └── run pending SQL migrations
   └── set PRAGMA journal_mode=WAL, foreign_keys=ON

3. File discovery (scanner module)
   └── use `ignore` crate: respects .gitignore, .scopeignore, --ignore-file
   └── filter: only files with extensions supported by available adapters
   └── filter: skip .scope/ directory itself
   └── result: Vec<(PathBuf, Metadata)>

4. Content hashing (parallel, rayon)
   └── for each file: blake3::hash(file_contents) → hex string
   └── compare against files.hash in DB
   └── partition: Vec<changed_files>, Vec<unchanged_files>
   └── unchanged_files: skip entirely (no parse, no DB write)

5. Parallel parsing (rayon ThreadPool, default: num_cpus threads)
   └── for each changed file:
       a. read file contents (UTF-8 with fallback to lossy conversion)
       b. dispatch to appropriate language adapter by extension
       c. adapter.extract(path, source) → ExtractResult
       d. on panic: catch and record as ParseError with status="error"
   └── result: Vec<(PathBuf, ExtractResult)>

6. Resolution pass (sequential, accesses DB for cross-file resolution)
   └── for each ImportRecord with ImportPath::Relative(partial):
       a. normalize: resolve `./foo` relative to file's directory
       b. try extensions in priority order: .ts, .tsx, .js, .rs, etc.
       c. look up resolved path in files table (must already exist)
       d. if found: set resolved_file_id, certainty=exact (for direct) or resolved
       e. if not found: leave resolved_file_id=NULL, certainty=unresolved
   └── for each CallSiteRecord:
       a. try to resolve callee_name in: same file symbols, imported symbols
       b. if found in same file: exact
       c. if found via import: resolved
       d. if not found: heuristic (call exists but target unknown)

7. DB write (transaction per file batch)
   └── BEGIN TRANSACTION
   └── for each changed file:
       a. UPSERT files row (path, language, hash, mtime, parse_status, indexed_at)
       b. DELETE old symbols, imports, edges (CASCADE)
       c. INSERT symbols (from ExtractResult.symbols)
       d. INSERT imports (from ExtractResult.imports)
       e. INSERT file_edges derived from resolved imports
       f. INSERT symbol_edges from resolved call sites
   └── COMMIT
   └── on error: ROLLBACK, retry individual files, log failures

8. index_meta update
   └── UPDATE index_meta SET value=... for: last_index_at, total_files, total_symbols

9. Git log population (optional, async after main index)
   └── if git is available AND --no-git not set:
       a. run: git log --name-only --since=<window> --format="%H %at %ae"
       b. parse: commit_sha, timestamp, author, list of changed files
       c. normalize file paths to repo-root-relative
       d. INSERT INTO file_churn (file_id, commit_sha, author, timestamp)
            ON CONFLICT(file_id, commit_sha) DO NOTHING
       e. compute co-change pairs for new commits:
            for each commit with ≥2 files: increment shared_commits for each pair
       f. INSERT/UPDATE file_cochange with new co-change rates

10. Capability tag loading (if arch.toml has [[capability]] sections)
    └── DELETE FROM capabilities (full rebuild from arch.toml)
    └── for each [[capability]] section:
        a. if `pattern` set: resolve glob against file list, INSERT file_pattern rows
        b. if `symbols` set: INSERT symbol_name rows
```

### Incremental index pipeline

Incremental index is triggered by default on every `scope index` call after the first.

```
1. Open DB, load current file hashes from `files` table into a HashMap<path, hash>

2. Walk repo files (same discovery as full index)

3. For each file:
   a. compute blake3 hash
   b. compare to stored hash
   c. if UNCHANGED: skip entirely (majority of files in normal dev)
   d. if CHANGED: add to re-index queue
   e. if NEW (not in DB): add to re-index queue
   f. if DELETED (in DB but not on disk):
      DELETE FROM files WHERE path=? (CASCADE removes all related rows)

4. For the re-index queue:
   same steps 5–8 as full index, but operating on only the changed files

5. Cross-file edge re-evaluation:
   After re-indexing changed files, check if any OTHER files had unresolved imports
   pointing to a newly-indexed file — re-attempt resolution for those imports.
   This handles the case where file A previously couldn't resolve import of file B,
   but B was just added/changed.

6. git log incremental update:
   Only fetch commits since last_index_at (stored in index_meta).
   Incremental: git log --since=<last_index_at>
```

### Watch mode (post-MVP)

```rust
// Uses notify crate for OS-level file system events
let watcher = notify::recommended_watcher(|res| {
    match res {
        Ok(event) => {
            // debounce: wait 200ms after last event before re-indexing
            // then run incremental index on affected paths only
        }
        Err(e) => eprintln!("watch error: {e}"),
    }
})?;

watcher.watch(repo_root, RecursiveMode::Recursive)?;
```

Watch mode debounces file system events (200ms window) to avoid re-indexing mid-save.
Only affected files are re-indexed — not the full repo.

---

## 15. Query engine

### Architecture

The query engine lives in `scope-core/src/query.rs`. It provides high-level functions
that compose DB queries and in-memory graph operations. The `graph.rs` module owns the
petgraph `DiGraph` that is loaded from SQLite on demand and cached for the duration of
a query session.

```rust
// Graph is loaded from SQLite once per query session and held in memory.
// For CLI use: one process per command invocation, so always fresh.
// For scope serve: loaded at startup, refreshed when index changes.
pub struct GraphSession {
    pub file_graph:   DiGraph<FileId, EdgeData>,
    pub symbol_graph: DiGraph<SymbolId, EdgeData>,
    pub db:           Connection,
}

pub struct EdgeData {
    pub kind:      EdgeKind,
    pub certainty: Certainty,
    pub line:      Option<u32>,
}
```

### Query categories and implementations

#### Category 1: File graph queries

**`deps(file, direction, transitive, depth_limit)`**

```
Algorithm:
- If direction=forward: BFS from file node following file_edges (from→to)
- If direction=reverse: BFS from file node following file_edges (to→from)
- If transitive=false: depth_limit=1
- For each reached node: record distance, certainty (min over path), reason string
- Certainty of path = minimum certainty of all edges on path
```

**`transitive_deps(file, depth_limit)`**

Extended BFS with depth tracking. At each level, continue expansion only if
current_depth < depth_limit. Returns all nodes by distance level.

#### Category 2: Symbol inventory queries

**`symbols(file, filters)`**

Direct DB query: `SELECT * FROM symbols WHERE file_id=? AND ...filters`
No graph traversal needed — pure DB query.

**`public_surface(path_prefix)`**

```sql
SELECT s.*, f.path FROM symbols s
JOIN files f ON s.file_id = f.id
WHERE s.visibility = 'public' AND s.exported = 1
  AND f.path LIKE ?  -- path_prefix filter
ORDER BY f.path, s.qualname
```

#### Category 3: Call graph queries

**`callers(symbol, transitive, depth_limit)`**

```
- Load symbol node from symbol_graph by qualname
- BFS reverse over symbol_edges WHERE kind='call'
- For each reached symbol: record file, qualname, distance, certainty, call_line
```

**`callees(symbol, transitive, depth_limit)`**

```
- Load symbol node from symbol_graph by qualname
- BFS forward over symbol_edges WHERE kind='call'
```

#### Category 4: Impact analysis queries

See Section 12 for detailed traversal strategy by change_type.

**Key implementation note:** Impact queries may traverse both the file graph AND symbol
graph. When traversing the file graph for `rename`/`delete`, we include all symbols in
all affected files (since a file rename changes all import statements for that file).
When traversing the symbol graph for `body`/`signature`, we stay in the symbol graph.

#### Category 5: Path queries

**`why(from, to, max_paths)`**

```
Algorithm (Yen's k-shortest-paths):
1. Find NodeIndex for `from` and `to` in file_graph
2. If not found: return no_path result
3. Run Dijkstra from `from` with edge weights:
   exact/resolved = 1, heuristic = 2, dynamic = 3
4. If no path found: return no_path with suggestion to check connectivity
5. For each shortest path up to max_paths:
   a. Record all hops (file, edge_kind, line, certainty)
   b. Compute path certainty = minimum certainty of all edges
   c. Yen's algorithm: penalize edges on already-found paths and re-run
6. Return paths sorted by length (ascending), then certainty (desc)
```

#### Category 6: Context queries

**`minimum_context_set(targets, change_type, budget_tokens)`**

```
Algorithm:
1. For each target file/symbol:
   a. BFS reverse (callers/importers) up to depth 2 → "must read callers"
   b. BFS forward (callees/imports) up to depth 1 → "must read callees"
   c. Mark target itself as priority 1
2. Score each reached file:
   score = (100 / distance) * certainty_weight * relevance_weight
   certainty_weight: exact=1.0, resolved=0.8, heuristic=0.5, dynamic=0.2
   relevance_weight: 1.5 if file defines/contains target, 1.0 otherwise
3. Sort by score descending
4. Classify:
   score > 50: must_read
   score > 20: should_read
   else: skip
5. If budget_tokens set:
   estimate token count per file (line_count * 8 as approximation)
   truncate must_read list at budget, converting overflow to should_read
```

**`pack(target, change_type, budget_tokens)`**

```
Algorithm:
1. Run minimum_context_set to get ranked file list
2. Fetch relevant data from DB:
   - target's public surface (symbols, visibility, spans)
   - direct callers with line numbers
   - direct callees with certainty
   - transitive callers within budget
   - rename/delete: sites that need updating
3. Format as plain text with section headers
4. Count tokens with tiktoken-rs (cl100k_base encoding)
5. If over budget: truncate transitive sections first, then should_read sections
6. Prepend metadata header with token count, generation time, schema version
```

#### Category 7: Architectural queries

**`arch_violations(arch_config)`**

```
Algorithm: O(edges)
1. For each file_edge (from, to):
   a. find from_layer: first layer in arch_config where from.path matches pattern
   b. find to_layer: first layer where to.path matches pattern
   c. for each rule in arch_config:
      if rule.from == from_layer AND to_layer in rule.may_not_import:
        add to violations
2. Return violations with from_file, to_file, violated_rule, certainty
```

**`stability_metrics(files)`**

```
For each file f:
  fan_in  = COUNT(DISTINCT from_file_id) FROM file_edges WHERE to_file_id=f.id
  fan_out = COUNT(DISTINCT to_file_id)   FROM file_edges WHERE from_file_id=f.id
  I = fan_out / (fan_in + fan_out)   [if both=0: I=0]
  flagged = I > 0.5 AND fan_in > 10
```

**`risk_scores(window_days, threshold)`**

```
For each file f:
  transitive_dependents = BFS reverse count from f in file_graph
  commit_count = COUNT(*) FROM file_churn
    WHERE file_id=f.id AND timestamp > (now - window_days * 86400)
  risk = log(1 + transitive_dependents) * log(1 + commit_count)
Filter: only files where risk > threshold
Sort: risk DESC
```

#### Category 8: Temporal queries

**`cochange(file, threshold, window_days)`**

```
1. Load co-change pairs from file_cochange WHERE file_a_id=f.id OR file_b_id=f.id
   AND window_days=window_days
2. Filter: co_change_rate > threshold
3. For each pair: check if static edge exists in file_edges
4. Classify: expected (has static edge) vs unexpected (no static edge)
5. Sort by co_change_rate DESC
```

#### Category 9: Simulation queries

**`simulate_extract(symbols, from_file, into_file)`**

```
Algorithm — pure in-memory, no DB writes:
1. Clone file_graph and symbol_graph into temporary structures
2. Create hypothetical new file node for into_file in both graphs
3. For each extracted symbol:
   a. Find all caller symbols in symbol_graph
   b. In hypothetical graph: redirect call edges to new symbol nodes in into_file
   c. Add file_edge: from_file → into_file (source file now imports new module)
4. For each caller of extracted symbols:
   b. If that caller imported from_file primarily for these symbols:
      In hypothetical graph: redirect import edge from from_file to into_file
5. Run cycle detection on hypothetical file_graph
6. Compute stability metrics on hypothetical graph
7. Check arch violations on hypothetical graph
8. Diff against real graph: new edges, removed edges, metrics changes
9. Compute recommendation: high_value | medium_value | low_value | not_recommended
   Scoring: +2 per cycle resolved, +1 per instability improvement, -1 per violation added
10. Discard hypothetical graphs (no mutations to real data)
```

#### Category 10: Reachability queries

**`entry_points(user_designated)`**

```
If user_designated set (from arch.toml [[entry_point]] sections): use those.
Otherwise: find all file nodes with in-degree = 0 in file_graph
  (no file imports them — they are roots of the dependency graph)
Note: in large repos, many files may have in-degree 0 due to unresolved imports.
Heuristic: only count files with at least 1 outgoing import edge as entry points
  (pure leaf files with no imports are not real entry points).
```

**`reachability_cone(entry_point)`**

```
BFS forward from entry_point in file_graph.
Collect all reachable file nodes with distance from entry.
Time complexity: O(V + E)
```

**`unreachable_files()`**

```
1. Compute entry_points list
2. For each entry point: BFS forward → collect reachable set
3. Union all reachable sets
4. unreachable = all_files - reachable_union
5. For each unreachable file: fetch last_modified from filesystem (or mtime from DB)
6. Sort by last_modified_days_ago DESC (oldest first)
7. Label certainty:
   exact: TypeScript/statically-analyzed languages with full import resolution
   heuristic: JavaScript with potential dynamic require() edges
```

#### Category 11: Capability queries

**`capability_reach(capability_name)`**

```
1. Load capability_sources from capabilities table WHERE name=capability_name
2. For each source (file or symbol):
   Reverse BFS from source node in file_graph/symbol_graph
   Collect all files that can reach any capability source
3. Filter to only entry points
4. Separate expected (user-declared in arch.toml) vs unexpected
5. For unexpected: find shortest path from entry to capability source
6. Return grouped result
```

#### Category 12: Decomposition queries

**`split_suggestions(file, target_n)`**

```
1. Load all exported symbols of target file from symbols table
2. For each symbol: get its callers from symbol_graph
   → a set of caller_file_ids (the "caller profile" of this symbol)
3. Build a symmetric co-usage matrix:
   similarity[A][B] = |callers(A) ∩ callers(B)| / |callers(A) ∪ callers(B)|
   (Jaccard similarity between caller sets)
4. Greedy agglomerative clustering:
   while clusters > target_n:
     find pair (A, B) with highest Jaccard similarity
     merge A and B into one cluster
5. For each cluster: propose a module name based on:
   a. dominant calling directory (e.g., symbols only called from routes/ → "route-helpers")
   b. common prefix in symbol names
6. Compute stability improvement for each proposed split:
   run simulate_extract for each cluster → stability delta
7. Return ranked suggestions: highest stability improvement first
8. Include "remainder" cluster for symbols with broad/mixed caller sets
```

#### Category 13: Similarity queries

**`graph_signature_similarity(file_a, file_b)`**

```
Feature vector per file:
  F = {
    imported_module_names: Set<String>,      // normalized module names
    exported_symbol_kinds: MultiSet<String>, // {"function": 3, "class": 1}
    caller_directory_patterns: Set<String>,  // dirname of each caller, normalized
    fan_in_bucket: usize,   // 0=zero, 1=1-3, 2=4-10, 3=11+
    fan_out_bucket: usize,  // same buckets
  }

Jaccard similarity over set features, weighted average:
  sim = 0.3 * jaccard(imported_module_names_A, imported_module_names_B)
      + 0.3 * jaccard(exported_symbol_kinds_A, exported_symbol_kinds_B)
      + 0.2 * jaccard(caller_directories_A, caller_directories_B)
      + 0.1 * (1 - |fan_in_bucket_A - fan_in_bucket_B| / 3)
      + 0.1 * (1 - |fan_out_bucket_A - fan_out_bucket_B| / 3)
```

For the `scope mirror <file>` command (one vs all):
```
For each other file B:
  compute sim(A, B)
Filter: sim > threshold (default 0.7)
Sort: similarity DESC
```

For `scope mirror --all` (all pairs, O(N²)):
```
Use the same computation but for all N*(N-1)/2 pairs.
Only feasible on repos < ~1000 files without optimization.
For large repos: use locality-sensitive hashing (LSH) to find candidates first.
```

#### Category 14: Health and gate queries

**`compute_health_report(compare_snapshot)`**

```
Orchestration function — calls all other query modules:
1. coverage metrics: COUNT(imports) grouped by certainty
2. size metrics: COUNT(files), COUNT(symbols), AVG/MAX fan_in, fan_out
3. risk scores: top 10 + count by level
4. arch metrics: arch_violations count, cycles count, avg instability
5. dead code: entry.unreachable count, unused exports count
6. temporal coupling: cochange unexpected pairs count, god file candidates
   (files with fan_in > 20 AND fan_out > 20 — both high)
7. surface metrics: surface diff vs compare_snapshot
8. health score formula (see Section 16.17 for formula)
9. if compare_snapshot: load snapshot, compute all metrics for snapshot too, compute deltas
```

**`evaluate_gates(gates_config, compare_ref)`**

```
1. Load .scope/gates.toml
2. Compute current metric values (reuse health report metrics)
3. If compare_ref set: load that snapshot and compute delta metrics
4. For each gate:
   a. evaluate: current_value vs max/min_delta threshold
   b. if violation: record gate_result with severity, current_value, threshold, detail
5. Aggregate: errors = count severity=error violations, warnings = count severity=warning
6. Exit code: 1 if any errors, 0 if only warnings or none
   (with --strict: exit 1 if any warnings too)
```

### Explainability requirement

Every result returned by the query engine that represents an impact or traversal result
MUST include these fields in its data structure:

```rust
pub struct QueryResultNode {
    pub node:     NodeRef,       // file path or symbol qualname
    pub reason:   String,        // human-readable explanation of why this node appeared
    pub distance: u32,           // hop count from query target
    pub certainty: Certainty,    // minimum certainty over the path to this node
    pub edge_kind: EdgeKind,     // the type of edge that caused inclusion
}
```

The `reason` field must be a complete English sentence. Examples:
- "calls crate::resolver::resolve_symbol directly"
- "imports src/auth/middleware.js (which calls target)"
- "re-exports verify_token from src/auth/middleware.js"
- "dynamic dispatch path includes target module (certainty: dynamic)"


---

## 16. Feature specifications

### 16.1 `scope why <file-a> <file-b>` — Dependency Path Explanation

**Purpose:** Answer "why on earth does changing X break Y?" — the most frustrating
debugging question in large codebases. For agents, enables instant causality tracing
when an unexpected test fails after a change.

**User stories:**
- A developer changes `src/utils/logger.rs` and a test in `src/payments/` fails. They
  run `scope why src/utils/logger.rs src/payments/checkout.rs` and immediately see the
  import chain that connects them.
- An agent traces an unexpected impact result back to its root cause without reading files.

**CLI:**
```bash
scope why src/utils/logger.rs src/payments/checkout.rs
scope why crate::auth::verify_token crate::routes::checkout::process_payment
scope why <a> <b> --all-paths        # show up to 5 paths (default: 3)
scope why <a> <b> --json
```

**Algorithm (Yen's k-shortest-paths):**

```
1. Build or load GraphSession with file_graph (DiGraph<FileId, EdgeData>)
2. Find NodeIndex for 'from' and 'to' by path/qualname lookup
3. If either node not found: return clear error (file not indexed, run scope index)
4. Assign edge weights:
   exact    → weight = 1
   resolved → weight = 2
   heuristic → weight = 4
   dynamic  → weight = 8
   (lower weight = preferred path — shortest path prefers high-certainty edges)
5. Run Dijkstra from 'from' to find shortest path
6. If no path exists: return "no connection found" with suggestion to check:
   a. Are both files indexed? (scope doctor)
   b. Is the connection indirect? (try scope deps --transitive on one of them)
7. For each of up to max_paths paths (Yen's algorithm):
   a. Collect path: list of (node, incoming_edge_data) tuples
   b. Compute path certainty = minimum certainty of all edges on path
   c. Exclude path if it repeats any node (loop detection)
   d. Penalize edges used in prior paths and re-run Dijkstra
8. Annotate each hop with: file path, edge_kind, line number, certainty, import_text
9. Sort paths: shortest first, then highest certainty
```

**Human-readable output:**
```
$ scope why src/utils/logger.js src/routes/payments.js

Path 1 (length 3, certainty: resolved)
  src/utils/logger.js
    → imported by  src/services/stripe.js         line 4  [exact]
    → imported by  src/routes/payments.js          line 2  [exact]

Path 2 (length 4, certainty: heuristic)
  src/utils/logger.js
    → imported by  src/middleware/request.js       line 11 [exact]
    → imported by  src/services/webhook.js         line 7  [exact]
    → imported by  src/routes/payments.js          line 8  [heuristic]

No more paths found within depth limit.
```

**Edge cases:**
- Same file for both arguments: return "same file" error.
- File A and B connected only through dynamic/unresolved edges: return the path but
  annotate with `certainty: dynamic` and explain the limitation.
- Cycle in path: skip any path that revisits a node.
- Disconnected graph: if BFS exhausts all reachable nodes without finding `to`, return
  "no path" with the count of reachable nodes from `from`.

---

### 16.2 `scope context <task-description>` — Minimum Context Set

**Purpose:** Replace speculative file reading in agent workflows. Given a task description
or explicit targets, compute the minimum set of files a developer or agent must read to
complete the work safely.

**User stories:**
- Claude Code agent receives a task: "rename verify_token to validateToken in auth
  middleware". Instead of reading the whole codebase, it runs `scope context` and gets a
  ranked list of exactly which files need attention.
- A developer asks for the minimum context before making a change.

**CLI:**
```bash
scope context "rename verify_token in auth middleware and update all callers"
scope context --target src/auth/middleware.js --change-type rename
scope context --target src/auth/middleware.js --budget 8000
scope context --target src/auth/middleware.js src/routes/api.js --change-type signature
```

**Algorithm:**

Step 1 — Target identification:
- If `--target` flags given: use those directly.
- If free-text task: extract file/symbol references using simple keyword matching:
  - Quoted strings that match indexed file paths → file targets
  - CamelCase or snake_case identifiers that match symbol names → symbol targets
  - File-like tokens (contains `/` or `.rs`/`.ts`/`.js`) → file targets
- If no targets identified: return error asking user to use `--target`.

Step 2 — Context expansion via BFS:
```
must_read = {target}
should_read = {}

# Callers (who needs to know about the change)
callers_depth1 = reverse_BFS(targets, depth=1, change_type)
callers_depth2 = reverse_BFS(targets, depth=2, change_type)

# Callees (what does the target depend on — to understand it)
callees_depth1 = forward_BFS(targets, depth=1)

must_read += callers_depth1
must_read += callees_depth1
should_read += callers_depth2
```

Step 3 — Scoring:
```
For each file f in (must_read ∪ should_read):
  score(f) = base_score / distance
            * certainty_weight(min_certainty_on_path)
            * type_weight(f)

  certainty_weight: exact=1.0, resolved=0.8, heuristic=0.5, dynamic=0.2
  type_weight: target_file=2.0, defines_target_symbol=1.8, test_file=1.3,
               direct_caller=1.5, direct_callee=1.2, other=1.0
```

Step 4 — Budget application (if `--budget <tokens>` given):
```
For each file: estimate_tokens = line_count * 8 (rough estimate)
Sort must_read by score descending
Accumulate tokens until budget exceeded; move remainder to should_read
Sort should_read by score descending; truncate at 2x budget remainder
```

Step 5 — Classification:
```
must_read:   score > 50 OR distance = 1 OR is target file
should_read: score > 15
skip:        all other indexed files (with reason "no connection to task graph")
```

**Output notes:**
- `skip` list is a summary count, not an exhaustive list (too many files in real repos).
- Token estimates are labeled as approximate.
- Agents should treat `must_read` as mandatory before making any edits.

---

### 16.3 `scope pack <target> --budget <tokens>` — Agent Context Pack

**Purpose:** Generate a single pre-formatted plain-text document for direct injection into
an LLM context window. Unlike `scope context` (which lists files to read), `scope pack`
generates the actual pre-formatted payload.

**Design philosophy:** Agents spend tokens. Every byte of context pack must be high-signal.
No filler, no preamble, no formatting noise. Optimized for `cl100k_base` token counting.

**CLI:**
```bash
scope pack src/auth/middleware.js --budget 4000
scope pack crate::resolver::resolve_symbol --budget 2000 --change-type rename
scope pack src/auth/ --budget 6000                # pack a directory
scope pack src/auth/middleware.js --budget 4000 --output context.txt
```

**Pack content structure (in order of priority):**

```
1. Metadata header (always included, ~50 tokens)
   - target, change_type, budget, used tokens, timestamp

2. Target's public surface (~100-200 tokens depending on symbol count)
   - All exported symbols: name, kind, visibility, line number

3. Direct callers (certainty: exact/resolved only)
   - File path, import line, call site line(s)
   - Truncated at budget if necessary

4. Direct callees (what target calls)
   - Symbol name, target file, line, certainty

5. Transitive callers (distance 2)
   - Only if budget permits after steps 1-4
   - Summarized: "N additional transitive callers at distance 2 (budget exceeded)"

6. Change-type-specific content:
   rename: list of all sites needing update (import sites + call sites + re-exports)
   delete: same as rename but labeled "will break at"
   signature: list of callers that must be verified for signature compatibility
   body: list of test files that exercise the target
   visibility: list of external consumers that lose access

7. Footer: "END SCOPE PACK | schema: 1 | truncated: yes/no"
```

**Token counting:** Uses `tiktoken-rs` with `cl100k_base` encoding (same as GPT-4/Claude
tokenizer approximation). Actual pack output may vary by 5–10% from estimate due to
encoding differences.

**Budget enforcement:** Hard budget. Pack generation stops adding content once budget is
reached. The footer always reports whether truncation occurred.

---

### 16.4 `scope arch` — Architectural Layer Violation Detection

**Purpose:** Enforce user-defined architectural rules as a CI gate. Allow teams to codify
"utils must never import services" as a machine-checked rule rather than a code review
comment.

**Configuration file: `.scope/arch.toml`**

```toml
# Architectural layers — define named groups of files by glob pattern
[[layer]]
name = "routes"
pattern = "src/routes/**"
description = "HTTP route handlers — entry points, no business logic"

[[layer]]
name = "services"
pattern = "src/services/**"
description = "Business logic — may import models and utils"

[[layer]]
name = "models"
pattern = "src/models/**"
description = "Data models — may import utils only"

[[layer]]
name = "utils"
pattern = "src/utils/**"
description = "Pure utilities — must not import anything above them"

# Rules — specify forbidden import directions
[[rule]]
from = "utils"
may_not_import = ["routes", "services", "models"]
message = "utils must be pure — no business logic dependencies"

[[rule]]
from = "models"
may_not_import = ["routes", "services"]
message = "models must not import higher-level layers"

[[rule]]
from = "services"
may_not_import = ["routes"]
message = "services must not import routes — avoid circular dependency risk"

# Test file configuration
[tests]
patterns = [
  "**/*.test.*",
  "**/*.spec.*",
  "tests/**",
  "**/test_*.rs",
  "**/__tests__/**"
]

# Capability tags for scope audit
[[capability]]
name = "network"
pattern = "src/http/**"
symbols = ["axios.get", "axios.post", "fetch", "https.request"]
expected_callers = ["src/server.js", "src/workers/**"]

[[capability]]
name = "db-write"
symbols = ["db.exec", "db.run", "Model.save", "Model.create", "Model.delete"]
expected_callers = ["src/services/**", "src/scripts/**"]

[[capability]]
name = "exec"
symbols = ["child_process.exec", "child_process.spawn", "Command::new"]
expected_callers = ["src/cli.js", "src/scripts/**"]

# Designated entry points (for scope entry)
[[entry_point]]
pattern = "src/server.*"

[[entry_point]]
pattern = "src/cli.*"

[[entry_point]]
pattern = "src/scripts/**"
```

**`scope arch check` algorithm:**

```
1. Load arch.toml layers and rules
2. For each file_edge (from_file, to_file) in file_graph:
   a. find from_layer: first [[layer]] where from_file.path matches pattern
   b. find to_layer: first [[layer]] where to_file.path matches pattern
   c. if either file matches no layer: skip (unlayered files are not checked)
   d. for each rule: check if from_layer == rule.from AND to_layer in rule.may_not_import
   e. if violation: record {from_file, to_file, from_layer, to_layer, violated_rule}
3. Return all violations
```

**`scope arch init` algorithm:**

Auto-detects common directory-based layer conventions:

```
1. Walk top-level directory structure (depth 1-2)
2. For each directory matching common names:
   "routes", "controllers", "views"  → layer "routes"
   "services", "use-cases"           → layer "services"
   "models", "entities", "domain"    → layer "models"
   "utils", "helpers", "lib"         → layer "utils"
   "middleware"                       → layer "middleware"
   "config"                           → layer "config"
3. Generate standard rules based on detected layers
4. Write starter arch.toml with all generated layers and rules
5. Print: "Generated .scope/arch.toml — review and customize before using in CI"
```

**`scope arch explain <file>`:**

Shows which layer(s) a file belongs to, which rules apply to it, and which imports
are/would be violations. Useful for understanding why a violation was flagged.

**Human-readable output:**
```
$ scope arch check
VIOLATION  src/utils/token.js → src/services/user.js
  Rule:    utils.may_not_import = ["routes", "services", "models"]
  Message: utils must be pure — no business logic dependencies
  Line:    src/utils/token.js:3  import { findUser } from '../services/user'

Found 1 violation. Exit code: 1.
Run `scope arch check --json` for machine-readable output.
Run `scope arch explain src/utils/token.js` to see all rules for this file.
```

---

### 16.5 `scope risk [--days N]` — Churn-Weighted Blast Radius Score

**Purpose:** Identify the most dangerous files to touch right now — combining static
dependency fan-in (blast radius) with recent change frequency (churn rate). High churn +
high fan-in = "this file breaks often and when it does, everything breaks."

**Formula:**
```
risk_score(f) = log₂(1 + transitive_dependents(f)) × log₂(1 + commit_count(f, window))

Where:
  transitive_dependents = count of all files in reverse BFS cone of f
  commit_count = count of commits touching f in the last N days
  log₂ dampens extreme outliers while preserving relative order
```

**Risk levels:**
```
critical: risk_score > 80
high:     risk_score > 50
medium:   risk_score > 25
low:      risk_score ≤ 25
```

**CLI:**
```bash
scope risk                          # all files, sorted by score, 90-day default
scope risk --days 30                # shorter window (more recent churn)
scope risk --days 365               # longer window (historical pattern)
scope risk --file src/auth/token.js # detailed breakdown for one file
scope risk --threshold 50           # only CRITICAL and HIGH
scope risk --top 20                 # top 20 riskiest files only
scope risk --json
```

**Data sources:**
- `transitive_dependents`: computed by BFS over file_graph (cached in memory during session)
- `commit_count`: queried from `file_churn` table (populated during `scope index`)
- If git log was not populated (`--no-git` or git unavailable): risk = log₂(1 + fan_in)
  only, labeled "git unavailable — score based on fan-in only"

**`scope risk --file <path>` detailed output:**
```
src/auth/token.js — Risk: 94 (CRITICAL)
  Transitive dependents:  47  (directly: 12, via 2+ hops: 35)
  Commits in 90 days:     23  (last: 2 days ago, by: alice@example.com)
  Risk formula:           log₂(48) × log₂(24) = 5.58 × 4.58 = 25.6 → scaled to 94

  Top 5 files most affected if this changes:
    src/routes/api.js           distance: 1   certainty: exact
    src/middleware/auth.js      distance: 1   certainty: exact
    src/workers/sync.js         distance: 2   certainty: resolved
    src/scripts/migrate.js      distance: 2   certainty: resolved
    src/tests/integration.js    distance: 2   certainty: exact

  Recommendation: This file is high-risk. Consider:
    - scope test-map covers src/auth/token.js  (run these tests after any change)
    - scope stability --file src/auth/token.js  (check if it can be split)
```

---

### 16.6 `scope stability` — Module Instability Index

**Purpose:** Surface structural liabilities — files with many dependents (high fan-in)
that also depend on many other files (high fan-out). These are the hardest files to change
safely because any of their dependencies can break them, and when they break, many things
downstream break too.

**Formula (Robert C. Martin's Instability Metric):**
```
I(f) = fan_out(f) / (fan_in(f) + fan_out(f))

Where:
  fan_in  = number of files that directly import f (direct reverse dependents)
  fan_out = number of files that f directly imports (direct dependencies)

I = 0: maximally stable (only depended upon, depends on nothing)
I = 1: maximally unstable (depends on much, nobody depends on it)
```

**Categories:**
```
I ≈ 0, fan_in > 5    → "stable abstraction" — ideal for shared interfaces (good)
I ≈ 1, fan_in ≈ 0    → "healthy leaf node" — normal, low-risk (good)
I > 0.5, fan_in > 10 → "UNSTABLE AND CENTRAL" — structural liability, flag these
I = 0.5 ± 0.15, any  → "balanced" — watch but not critical
fan_in = 0, fan_out = 0 → "isolated file" — possibly dead code
```

**CLI:**
```bash
scope stability                          # all files, sorted by instability score
scope stability --file src/db/conn.js    # single file with detailed breakdown
scope stability --flag-threshold 0.5     # only files with I > 0.5 AND fan_in > 10
scope stability --sort fan-in            # sort by fan-in instead of instability
scope stability --json
```

**Human-readable output:**
```
$ scope stability

FLAGGED: Unstable and central (I > 0.5, fan_in > 10)
  src/db/connection.js       I: 0.71  fan_in: 29  fan_out: 71  ← STRUCTURAL LIABILITY
  src/auth/middleware.js     I: 0.62  fan_in: 18  fan_out: 30

Stable abstractions (I < 0.2, fan_in > 5)  ✓
  src/utils/constants.js     I: 0.02  fan_in: 48  fan_out:  1
  src/types/index.ts         I: 0.08  fan_in: 34  fan_out:  3

Summary: 2 flagged, 12 stable abstractions, 198 healthy leaves, avg I: 0.41
```

**Implementation:**
Pure arithmetic over existing fan-in/fan-out counts from file_edges. No new data
collection needed. Two SQL queries:
```sql
-- fan_in per file
SELECT to_file_id, COUNT(DISTINCT from_file_id) as fan_in FROM file_edges GROUP BY to_file_id;
-- fan_out per file
SELECT from_file_id, COUNT(DISTINCT to_file_id) as fan_out FROM file_edges GROUP BY from_file_id;
```
Join on file_id, compute I, categorize. Sub-millisecond for any repo size.

---

### 16.7 `scope surface` — Public API Surface Diff

**Purpose:** Automated semver assistant. Know exactly which public symbols changed between
two git refs, and whether that constitutes a breaking change, a minor addition, or a patch.

**CLI:**
```bash
scope surface                           # show current public surface
scope surface --path src/auth/          # surface for a subtree only
scope surface diff HEAD~1 HEAD          # what changed in the last commit?
scope surface diff main feature/auth    # compare branches
scope surface diff v1.2.0 v1.3.0        # compare tags/snapshots
scope surface --json
scope surface diff HEAD~1 HEAD --json
```

**What constitutes a "public symbol":**
- Rust: `pub fn`, `pub struct`, `pub enum`, `pub type`, `pub const`, `pub trait` at the
  crate root or in a `pub mod`
- TypeScript: any `export` declaration (function, class, interface, type, const)
- JavaScript: any `module.exports` or `export` at top level

**`scope surface diff` algorithm:**

```
Inputs: ref1, ref2

Option A — if both refs have named snapshots:
  Load snapshot ref1 edge_json (decompressed)
  Load snapshot ref2 edge_json
  Extract symbol lists from both snapshots
  Compute: removed = symbols_in_ref1 - symbols_in_ref2
           added   = symbols_in_ref2 - symbols_in_ref1
           changed = symbols_in_both with different signature/visibility

Option B — if refs are git branch/commit refs (no snapshots):
  Use git stash/checkout approach:
    git stash (save working changes)
    git checkout ref1 && scope index --no-git (reindex at ref1)
    capture current symbol table
    git checkout ref2 && scope index --no-git (reindex at ref2)
    capture current symbol table
    git stash pop (restore working changes)
  Compute diff between two symbol tables
  NOTE: This is slow and modifies the working tree — always warn user

Option C — preferred for CI:
  Require both refs to have pre-saved snapshots (scope snapshot save before each release)
  If snapshots not found: error with instructions to save them

Semver recommendation logic:
  if removed.count > 0: "breaking change (major)" — any removal is breaking
  elif changed.count > 0 AND changes_include_signature: "breaking change (major)"
  elif added.count > 0: "minor — new symbols added, no removals"
  else: "patch — no surface changes"
```

**Human-readable output:**
```
$ scope surface diff v1.2.0 v1.3.0

REMOVED (breaking):
  crate::auth::verify_token           function  public
    was: verify_token(token: &str) -> bool

ADDED:
  crate::auth::VerifyOpts             struct    public
  crate::auth::verify_token_with_opts function  public
    now: verify_token_with_opts(token: &str, opts: VerifyOpts) -> Result<Claims>

UNCHANGED: 312 public symbols unchanged

Semver recommendation: MINOR (1 removed, 2 added)
Note: The removed symbol may break external consumers. Verify this is intentional.
If external consumers exist, this should be treated as MAJOR.
```

---

### 16.8 `scope rename-plan <old> <new>` — Safe Rename Execution Plan

**Purpose:** Replace grep-and-replace (which creates broken intermediate states) with a
topologically-ordered, line-precise execution plan that handles every rename site in the
correct order.

**CLI:**
```bash
scope rename-plan verifyToken validateToken        # dry run (default)
scope rename-plan src/auth/middleware.js src/auth/verify.js  # file rename
scope rename-plan verifyToken validateToken --apply   # execute
scope rename-plan verifyToken validateToken --apply --force  # include uncertain sites
scope rename-plan verifyToken validateToken --json   # machine-readable plan
```

**Topological ordering:**
```
Order of operations for renaming symbol 'old' → 'new':
  Step 1: Update definition site(s) in defining file
  Step 2: Update re-export sites (files that re-export the symbol)
  Step 3: Update import sites (files that import by name)
  Step 4: Update call sites (files that call the symbol)
  Step 5: Update string literals and comments (heuristic — labeled uncertain)

Rationale: If we update callers before the definition, any intermediate
state has callers using a name that doesn't exist yet. Top-down ordering
ensures the symbol always exists under some name at each step.
```

**Algorithm:**
```
1. Resolve 'old' to a symbol or file in the DB
2. Collect all sites:
   a. definition_sites: spans in symbols table (span_start, span_end)
   b. re_export_sites: symbol_edges WHERE kind='re-export' AND to_symbol=old_sym
   c. import_sites: imports table WHERE raw_text contains old_name
   d. call_sites: symbol_edges WHERE kind='call' AND to_symbol=old_sym
   e. string_sites: grep source files for old_name as string literal (heuristic)
   f. doc_comment_sites: grep for old_name in doc comment regions (heuristic)

3. Annotate certainty:
   definition_sites → exact
   re_export_sites  → exact
   import_sites     → exact (for static imports) or heuristic (for dynamic)
   call_sites       → exact or resolved (based on symbol resolution certainty)
   string_sites     → heuristic
   doc_comment_sites → heuristic

4. Order sites by step (definition first, then re-exports, then imports, then calls)
5. Within each step: sort by file path (alphabetical), then by line number

6. Generate substitution list:
   For each site: {file, start_byte, end_byte, old_text, new_text}

7. Output:
   Exact sites (will be applied with --apply)
   Uncertain sites (shown but require --force to apply)
```

**`--apply` mode — safe file rewriter:**
```
For each file with at least one substitution:
  1. Read file contents into memory
  2. Sort substitutions by start_byte DESCENDING (apply from end to start)
     Rationale: applying from end to start preserves byte offsets of earlier sites
  3. For each substitution:
     a. Verify: contents[start_byte..end_byte] == old_text (sanity check)
     b. If mismatch: abort for this file, report mismatch (index may be stale)
     c. Apply: replace bytes start_byte..end_byte with new_text
  4. Verify resulting file is valid UTF-8
  5. Write to disk atomically (write temp file, rename into place)
  6. Update index for this file (scope index on just this file)
```

**Dry-run output:**
```
$ scope rename-plan verifyToken validateToken

RENAME PLAN: verifyToken → validateToken
14 sites total (12 exact, 2 uncertain)

Step 1 — Definition (1 site)
  src/auth/middleware.js:12   function verifyToken(  →  function validateToken(

Step 2 — Re-exports (0 sites)

Step 3 — Import sites (5 sites)
  src/routes/api.js:3         import { verifyToken }  →  import { validateToken }
  src/routes/admin.js:2       import { verifyToken }  →  import { validateToken }
  src/tests/auth.test.js:4    import { verifyToken }  →  import { validateToken }
  src/app.js:7                import { verifyToken }  →  import { validateToken }
  src/workers/sync.js:1       import { verifyToken }  →  import { validateToken }

Step 4 — Call sites (6 sites)
  src/routes/api.js:14        verifyToken(req, res  →  validateToken(req, res
  src/routes/api.js:28        router.use(verifyToken  →  router.use(validateToken
  ... (4 more)

Step 5 — Uncertain: string literals (2 sites, requires --force)
  src/docs/api.md:47          "verifyToken"  ← string literal, may be intentional
  src/config/swagger.yml:89   verifyToken    ← YAML value, may be intentional

Run with --apply to execute steps 1-4.
Run with --apply --force to also execute step 5.
```

---

### 16.9 `scope test-map` — Static Test Coverage Topology

**Purpose:** Know which tests cover a file before touching it — without running any tests.
Purely static, based on import graph analysis. Works in any language, any test framework.

**CLI:**
```bash
scope test-map build                         # detect test files, build coverage map
scope test-map covers src/auth/middleware.js # which tests cover this file?
scope test-map covered-by tests/auth.test.js # what does this test cover?
scope test-map uncovered                     # source files with no test coverage in graph
scope test-map --json
```

**Test file detection:**
Test files are identified by patterns in `.scope/arch.toml` `[tests]` section (configurable):
```
Default patterns:
  **/*.test.ts, **/*.test.js, **/*.test.rs
  **/*.spec.ts, **/*.spec.js
  tests/**/*.ts, tests/**/*.js, tests/**/*.rs
  **/test_*.rs
  **/__tests__/**
  src/**/__mocks__/**  (excluded — not real tests)
```

**`test-map build` algorithm:**
```
1. Identify all test files by matching against patterns
2. For each test file T:
   BFS forward from T in file_graph (T imports → imports of imports → ...)
   Every source file reachable from T is "covered by T"
3. Build coverage_map: Map<source_file, Set<test_files>>
4. Store result in memory (not persisted — recomputed from live graph on demand)
```

**`test-map covers <source-file>` algorithm:**
```
reverse lookup: coverage_map[source_file] → Set<test_files>
Sort test files by: how directly they import the source (distance)
```

**`test-map uncovered` algorithm:**
```
all_source_files = file_graph.nodes WHERE file NOT in test_file_patterns
covered_files = union of all coverage_map values
uncovered = all_source_files - covered_files
Sort by: last_modified (show recently-modified uncovered files first)
```

**Key use cases:**
1. Before deleting a source file: `scope test-map covers src/utils/legacy.js` — if empty,
   safe to delete with no test impact.
2. After changing a file: run only the tests returned by `covers` — no full test suite
   needed for small changes.
3. Finding dead test files: tests that cover zero source files (all their imports are
   test infrastructure, not source).
4. CI optimization: compute test set based on files changed in the diff.

**Human-readable output:**
```
$ scope test-map covers src/auth/middleware.js

Tests covering src/auth/middleware.js (3 test files):
  tests/auth/middleware.test.js     distance: 1  (imports directly)
  tests/integration/api.test.js     distance: 2  (imports via routes/api.js)
  tests/e2e/checkout.test.js        distance: 4  (imports via long chain)

Recommendation: Run these 3 test files after changing src/auth/middleware.js.
```

---

### 16.10 `scope snapshot` + `scope diff-snapshot` — Architectural Time Travel

**Purpose:** Track how the *architecture itself* evolves over time — not just which lines
changed, but whether the system is becoming more or less coupled, gaining or losing
abstractions, or accumulating layer violations. Enables trend-based CI gates.

**CLI:**
```bash
scope snapshot save --name v1.2.0                   # save current graph
scope snapshot save --name v1.2.0 --commit HEAD     # tag with git SHA
scope snapshot list                                  # list saved snapshots
scope snapshot delete v1.0.0                         # delete old snapshot
scope diff-snapshot v1.2.0 v1.3.0                   # architectural diff
scope diff-snapshot v1.2.0 v1.3.0 --json
```

**Snapshot format:**

Snapshots are stored as zstd-compressed JSON in the `snapshots` table `edge_json` column.
JSON structure:
```json
{
  "schema_version": 1,
  "snapshot_version": 1,
  "created_at": 1741420800,
  "files": [
    { "id": 1, "path": "src/auth/middleware.js", "hash": "abc123" }
  ],
  "symbols": [
    { "id": 1, "file_id": 1, "qualname": "auth::verifyToken", "kind": "function",
      "visibility": "public", "exported": true }
  ],
  "file_edges": [
    { "from": 1, "to": 2, "kind": "import", "certainty": "exact" }
  ],
  "symbol_edges": [
    { "from": 1, "to": 3, "kind": "call", "certainty": "exact" }
  ]
}
```

Compression: `zstd::encode_all(json_bytes, 3)`. Typical compression ratio 8:1 for edge
lists. A 100K-file repo graph might compress from 50MB to 6MB.

**`diff-snapshot` algorithm:**
```
1. Load and decompress snapshots A and B
2. Rebuild edge sets from both snapshots (use normalized (from_path, to_path, kind) tuples)
3. Compute set differences:
   added_edges   = edges_B - edges_A
   removed_edges = edges_A - edges_B
4. Identify new central dependencies:
   For each file F: compare fan_in(F, snapshot_A) vs fan_in(F, snapshot_B)
   Report files where fan_in increased by > 5 (newly central)
5. Cycle detection on both snapshots:
   count cycles in A, count cycles in B
   new_cycles = cycles_B - cycles_A (set difference on cycle members)
6. Layer violation re-evaluation:
   Re-apply current arch.toml rules to both edge sets
   violations_A, violations_B
   introduced = violations_B - violations_A
   resolved   = violations_A - violations_B
7. Stability delta:
   compute avg instability for both snapshots
   delta = avg_I(B) - avg_I(A)
8. Surface diff: extract public symbols from both snapshots, diff as in scope surface diff
```

**The most valuable CI use case:**
```bash
# In CI: on every PR, save a snapshot of the current branch
scope snapshot save --name "pr-${PR_NUMBER}"
# Compare against main branch snapshot
scope diff-snapshot main "pr-${PR_NUMBER}" --json | jq '.layer_violations.introduced'
# Fail CI if new violations introduced
```

---

### 16.11 `scope cochange` — Temporal Coupling Detection

**Purpose:** Discover *hidden coupling* — pairs of files that always change together in git
commits despite having no static import relationship. This reveals shared concerns,
copy-paste patterns, or implicit conventions that the static graph cannot see.

**Why temporal coupling matters:**
A static dependency graph answers "what *must* change together?" Temporal coupling answers
"what *does* change together in practice?" The gap between these two is where hidden
architectural debt lives.

**CLI:**
```bash
scope cochange                          # all unexpected pairs, 90-day window
scope cochange --file src/auth/middleware.js    # partners for one file
scope cochange --compare-static         # only pairs without any static import edge
scope cochange --threshold 0.6          # pairs co-changing 60%+ of commits
scope cochange --days 30                # 30-day window (recent changes only)
scope cochange --json
```

**Algorithm:**

Step 1 — Build commit-to-files map (during `scope index --git`):
```sql
-- Already done: file_churn table has (file_id, commit_sha, timestamp)
-- Group by commit_sha to get all files changed in each commit:
SELECT commit_sha, GROUP_CONCAT(file_id) as file_ids
FROM file_churn
WHERE timestamp > (? - window_days * 86400)
GROUP BY commit_sha
HAVING COUNT(*) >= 2   -- only commits with 2+ files
```

Step 2 — Build co-occurrence matrix:
```
For each commit C with files [f1, f2, f3, ...]:
  For each pair (fi, fj) where i < j:
    cochange_count[min(fi,fj)][max(fi,fj)] += 1
    total_commits[fi] += 1
    total_commits[fj] += 1
```

Step 3 — Compute co-change rates and store:
```
For each pair (A, B):
  rate_A_given_B = cochange_count[A,B] / total_commits[B]
  rate_B_given_A = cochange_count[A,B] / total_commits[A]
  co_change_rate = max(rate_A_given_B, rate_B_given_A)
  -- store max: "when A changes, B changes 89% of the time"

INSERT INTO file_cochange
  (file_a_id, file_b_id, shared_commits, total_commits_a, total_commits_b, window_days)
```

Step 4 — Cross-reference with static edges:
```
For each pair in file_cochange WHERE co_change_rate > threshold:
  has_static_edge = EXISTS(
    SELECT 1 FROM file_edges
    WHERE (from_file_id=A AND to_file_id=B) OR (from_file_id=B AND to_file_id=A)
  )
  classification = has_static_edge ? "expected" : "unexpected"
```

**Human-readable output:**
```
$ scope cochange --compare-static --threshold 0.7

Unexpected temporal coupling (no static dependency, co-change ≥ 70%):

  src/auth/middleware.js ←→ src/config/constants.js
    Co-change: 89%   Shared commits: 34   Window: 90 days
    Suggestion: These files have no import relationship but almost always change together.
    Consider: Is constants.js a dependency that should be imported? Or should these
    constants be co-located in the auth module?

  src/services/payment.js ←→ src/services/email.js
    Co-change: 74%   Shared commits: 21   Window: 90 days
    Suggestion: These sibling services co-evolve. Are they implementing a shared protocol?

Found 7 unexpected co-change pairs. 4 expected (have static edges).
```

---

### 16.12 `scope simulate extract` — Refactoring Simulation

**Purpose:** Before doing any actual refactoring, show what the dependency graph would
look like if a set of symbols were extracted into a new module. Validate the improvement
hypothesis before touching any files.

**CLI:**
```bash
scope simulate extract verifyToken,createSession --into src/auth/session.js
scope simulate extract verifyToken,createSession --into src/auth/session.js --json
# After reviewing simulation, generate the execution plan:
scope rename-plan --from-simulate  # (reads last simulation result)
```

**Algorithm:**

```
Input: symbols=[S1,S2,...], from_file=F, into_file=NEW

1. Validate input:
   - All symbols must be in from_file (look up in symbols table)
   - into_file must not already exist in the index

2. Clone the in-memory graph (copy file_graph, symbol_graph — O(V+E))
   into: hypothetical_file_graph, hypothetical_symbol_graph

3. Add hypothetical new file node NEW to both graphs

4. For each extracted symbol Si:
   a. Find all callers of Si in symbol_graph (symbol_edges WHERE to=Si, kind=call)
   b. In hypothetical graphs:
      - Create new symbol node Si' in NEW
      - Redirect all call edges from callers → Si' instead of Si
      - Add import edge from F → NEW (source file now imports its own extraction)
   c. Find all import edges from other files importing F for Si specifically
      (heuristic: if caller only uses Si from F, they can redirect to NEW)
      - In hypothetical graph: add direct import edge from caller → NEW
      - If caller already had an import edge to F for other symbols: keep that edge too

5. Layer violation check on hypothetical_file_graph:
   Apply arch.toml rules to the hypothetical edge set
   new_violations = violations_hypothetical - violations_real

6. Cycle detection on hypothetical_file_graph:
   Run Tarjan's SCC on hypothetical_file_graph
   new_cycles = cycles_hypothetical - cycles_real
   resolved_cycles = cycles_real - cycles_hypothetical

7. Stability metrics on hypothetical_file_graph:
   Compute I(F) in real and hypothetical graphs
   Compute I(NEW) in hypothetical graph
   Other files affected: recompute I for all callers of Si

8. Compute recommendation score:
   +3 per cycle resolved
   +2 per instability improvement > 0.1
   +1 per file that gains a cleaner dependency
   -3 per cycle introduced
   -2 per layer violation introduced
   -1 per file that gains an extra dependency
   
   Score > 5: "high_value"
   Score 2-5: "medium_value"
   Score 0-1: "low_value"
   Score < 0: "not_recommended"

9. Discard hypothetical graphs (no DB writes)
```

**Chaining to rename-plan:**
After simulation shows a favorable outcome, the developer can run:
```bash
scope rename-plan --from-simulate
```
Which reads the last simulation result from a temporary file in `.scope/last_simulate.json`
and generates the actual file-editing plan.

---

### 16.13 `scope entry` — Entry Point and Reachability Analysis

**Purpose:** Find the true entry points of the codebase (roots of the dependency graph)
and compute what is reachable from each. The most actionable output is `unreachable` —
files provably reachable from no entry point are dead code.

**CLI:**
```bash
scope entry list                    # all entry points (zero in-degree nodes)
scope entry cone src/server.js      # everything reachable from this entry
scope entry reaches src/utils/jwt.js  # which entries can reach this file?
scope entry unreachable             # files reachable from no entry point
scope entry unreachable --min-age-days 90   # only files untouched for 90+ days
scope entry --json
```

**Entry point detection:**

Priority order:
1. Explicit: `[[entry_point]]` patterns in `.scope/arch.toml`
2. Inferred: file nodes with in-degree = 0 in file_graph AND fan_out > 0
   (imported by nothing internal, but imports other things)

```sql
-- Files with zero in-degree (no file imports them)
SELECT f.id, f.path FROM files f
WHERE NOT EXISTS (
  SELECT 1 FROM file_edges e WHERE e.to_file_id = f.id
)
AND EXISTS (
  SELECT 1 FROM file_edges e WHERE e.from_file_id = f.id
)
```

**Reachability cone algorithm:**
```
BFS forward from entry_point in file_graph:
  Start: {entry_point}
  Expand: all to_file_id nodes via file_edges
  Record: {file_id, distance, certainty (min over path)}
  Collect until BFS exhausts all reachable nodes
Time: O(V + E)
```

**`entry unreachable` algorithm:**
```
1. entry_points = entry_point_detection()
2. reachable = ∅
3. For each entry in entry_points:
   reachable ∪= reachability_cone(entry)
4. all_files = all file IDs in files table
5. unreachable = all_files - reachable
6. Enrich each unreachable file:
   - last_modified: mtime from DB or filesystem stat
   - exported_symbols: COUNT from symbols WHERE exported=1
   - certainty: "exact" for TypeScript/Rust, "heuristic" for JavaScript
7. Sort by last_modified ASC (oldest first — most likely dead)
```

**Important caveats displayed in output:**
- For dynamic languages (JavaScript): "Dynamic require() calls may create edges not
  captured by static analysis. These files may not be truly dead."
- For TypeScript/Rust: "Higher confidence — static imports are exhaustively detected."
- Always: "Verify by searching for string references before deleting."

**Human-readable output:**
```
$ scope entry list
Entry points (5 files with no internal importers):
  src/server.js             imports: 23  (web server)
  src/cli.js                imports: 11  (CLI entry)
  src/workers/queue.js      imports: 8   (background job runner)
  src/scripts/migrate.js    imports: 4   (database migration)
  src/scripts/seed.js       imports: 3   (database seeder)

$ scope entry unreachable
UNREACHABLE FILES (3) — certainty: heuristic
  src/utils/legacy_crypto.js    last changed: 847 days ago  exports: 4
  src/helpers/xml_parser.js     last changed: 203 days ago  exports: 7
  src/types/deprecated.ts       last changed: 1203 days ago exports: 12  [exact]

These files are not reachable from any detected entry point.
Verify before deleting: grep -r "legacy_crypto\|xml_parser\|deprecated" src/
```

---

### 16.14 `scope audit --capability` — Transitive Capability Reach

**Purpose:** For security reviews and compliance, trace which entry points can
transitively call sensitive operations (network, database writes, exec, filesystem).
Flags *unexpected* reach — entry points that reach a capability through a path the
developer didn't intend.

**CLI:**
```bash
scope audit --capability network      # which entries reach network I/O?
scope audit --capability db-write     # which entries reach DB writes?
scope audit --capability exec         # which entries can run subprocesses?
scope audit --surface                 # which public API symbols expose capabilities?
scope audit --capability network --json
```

**Configuration in `.scope/arch.toml`:**
```toml
[[capability]]
name = "network"
pattern = "src/http/**"             # all files in http/ are capability sources
symbols = ["fetch", "axios.request", "https.get"]  # specific function calls

# Declare which entries are EXPECTED to reach this capability
expected_callers = ["src/server.js", "src/workers/**"]
# Any entry NOT in expected_callers that reaches network → flagged as unexpected
```

**Algorithm:**
```
Input: capability_name

1. Load capability sources:
   - Files matching capability patterns
   - Symbols matching capability symbol names
   → Set<FileId | SymbolId>: capability_sources

2. Reverse BFS from each capability source:
   In file_graph: BFS reverse (following edges backward)
   Collect all files that can reach any capability source
   → reachable_from_capability: Map<FileId, ShortestPath>

3. Filter to entry points:
   entry_file_reach = entry_points ∩ reachable_from_capability

4. Classify each reaching entry point:
   Load expected_callers from arch.toml capability config
   Expand patterns to actual file IDs
   expected_entries = entry_file_reach WHERE file matches expected_callers
   unexpected_entries = entry_file_reach - expected_entries

5. For each unexpected entry: find shortest path to capability
   Run Dijkstra from entry to any capability source node
   Return path for the human-readable output

6. For --surface mode:
   Find all PUBLIC exported symbols that are in reachable_from_capability
   These are the surface-visible capability exposures
```

**Human-readable output:**
```
$ scope audit --capability network

Capability: network
Sources: 4 files (src/http/client.js, src/http/webhook.js, ...)

EXPECTED REACH:
  src/server.js          ✓  (declared expected)
  src/workers/queue.js   ✓  (declared expected)

UNEXPECTED REACH (2 entry points):
  src/cli.js
    Path: src/cli.js → src/utils/format.js → src/integrations/stripe.js
    Note: format utility imports stripe for currency — likely unintentional
    Fix: Move currency formatting logic to a non-network utility

  src/scripts/migrate.js
    Path: src/scripts/migrate.js → src/services/email.js → src/integrations/sendgrid.js
    Note: Migration triggers welcome emails — this is almost certainly a bug
    Fix: Pass a no-op email adapter in migration context

Recommendation: Review 2 unexpected network access paths before deployment.
```

---

### 16.15 `scope split <file>` — God-File Decomposition Suggestions

**Purpose:** When a file has grown too large and has too many unrelated symbols, suggest
a concrete decomposition into natural sub-modules by analyzing which symbols are always
used together and which are used by entirely different callers.

**CLI:**
```bash
scope split src/utils/helpers.js         # suggest natural decomposition
scope split src/utils/helpers.js --n 3   # suggest split into exactly 3 modules
scope split src/utils/helpers.js --json  # machine-readable for scripting
```

**Algorithm:**

Step 1 — Build caller profile per symbol:
```sql
-- For each exported symbol in target file,
-- get the set of CALLING FILES (not symbols — file-level coupling)
SELECT s.id, s.name, GROUP_CONCAT(DISTINCT f_caller.path) as callers
FROM symbols s
JOIN symbol_edges se ON se.to_symbol_id = s.id AND se.kind = 'call'
JOIN symbols s_caller ON s_caller.id = se.from_symbol_id
JOIN files f_caller ON f_caller.id = s_caller.file_id
WHERE s.file_id = ?   -- target file
GROUP BY s.id
```

Step 2 — Build Jaccard similarity matrix:
```
For each pair of symbols (A, B) in target file:
  callers_A = set of caller files of A
  callers_B = set of caller files of B
  similarity(A, B) = |callers_A ∩ callers_B| / |callers_A ∪ callers_B|

Symbols with similarity = 1.0: always called together from same files → same cluster
Symbols with similarity = 0.0: never called together → different clusters
```

Step 3 — Greedy agglomerative clustering:
```
Initialize: each symbol is its own cluster
Repeat until cluster_count == target_n OR all remaining pairs have similarity < 0.3:
  Find pair (cluster_A, cluster_B) with highest average pairwise similarity
  Merge A and B into a new cluster
  Recompute: similarity(new_cluster, X) = avg(similarity(member, X) for member in new_cluster)

Result: target_n clusters (or fewer if similarity is too low to justify merging)
```

Step 4 — Name suggestions:
```
For each cluster:
  Dominant caller directories = most common dirname among all callers of cluster symbols
  Symbol prefix = longest common prefix of symbol names (if meaningful > 3 chars)
  
  Suggested name = f"{dominant_dir_basename}-{symbol_prefix_or_theme}"
  Examples:
    symbols used only by routes/ → "route-helpers.js"
    symbols with prefix "format" → "formatters.js"
    symbols used by checkout/ only → "checkout-utils.js"
```

Step 5 — Stability projection:
```
For each proposed cluster:
  Run simulate_extract(cluster_symbols, from_file, proposed_new_file)
  Capture: instability_delta, cycles_resolved, violations_introduced
```

**Human-readable output:**
```
$ scope split src/utils/helpers.js

God-file detected: 67 exports, 14 distinct caller directories

Suggested decomposition (3 clusters):

Cluster A → src/utils/formatters.js
  Symbols (12): formatCurrency, formatDate, formatPhone, formatAddress, ...
  Callers: src/routes/ (9 files), src/views/ (3 files)
  Stability improvement: I: 0.71 → 0.38 for helpers.js  ✓
  Validate: scope simulate extract formatCurrency,formatDate,... --into src/utils/formatters.js

Cluster B → src/utils/validators.js
  Symbols (18): validateEmail, validatePhone, validateCreditCard, ...
  Callers: src/services/ (6 files), src/middleware/ (2 files)
  Stability improvement: I: 0.71 → 0.44 for helpers.js  ✓

Cluster C — Remainder → stays in helpers.js (or split further)
  Symbols (37): mixed usage, no clear grouping
  Suggestion: Consider keeping in helpers.js or running `scope split --n 5` for finer split

Run `scope simulate extract <symbols> --into <file>` to validate each cluster before refactoring.
```

---

### 16.16 `scope mirror <file>` — Structural Similarity Detection

**Purpose:** Find files with similar *architectural roles* — not similar source text, but
similar graph topology. Two payment service adapters with the same import/export structure
are candidates for a shared interface. Two configuration loaders with identical topology
might be consolidation candidates.

**CLI:**
```bash
scope mirror src/services/stripe.js          # find similar files in repo
scope mirror src/services/stripe.js src/services/paypal.js  # compare two
scope mirror --threshold 0.8                 # only 80%+ similar pairs
scope mirror --all                           # scan all pairs (slow for large repos)
scope mirror --json
```

**Feature vector construction:**

For each file F, build a feature vector:
```rust
struct GraphSignature {
  // What modules does this file import? (normalized: strip paths, keep package names)
  imported_module_names: BTreeSet<String>,  // {"express", "lodash", "pg"}
  // What kinds of symbols does this file export?
  exported_symbol_kinds: BTreeMap<SymbolKind, u32>,  // {Function: 3, Class: 1}
  // From which top-level directories do callers come?
  caller_directory_prefixes: BTreeSet<String>,  // {"routes", "tests", "middleware"}
  // Fan-in and fan-out bucketed
  fan_in_bucket: u8,   // 0=zero, 1=1-3, 2=4-10, 3=11+
  fan_out_bucket: u8,  // same
  // Does this file appear to be a test file?
  is_test: bool,
  // Language
  language: String,
}
```

**Similarity computation (weighted Jaccard):**
```
sim(A, B) =
  0.35 * jaccard(A.imported_module_names, B.imported_module_names)
+ 0.30 * kind_similarity(A.exported_symbol_kinds, B.exported_symbol_kinds)
+ 0.20 * jaccard(A.caller_directory_prefixes, B.caller_directory_prefixes)
+ 0.10 * bucket_similarity(A.fan_in_bucket, B.fan_in_bucket)
+ 0.05 * bucket_similarity(A.fan_out_bucket, B.fan_out_bucket)

Where:
  jaccard(S1, S2) = |S1 ∩ S2| / |S1 ∪ S2|  (0.0 if both empty → 0.5 neutral)
  kind_similarity: cosine similarity over kind frequency vectors
  bucket_similarity: 1 - |a - b| / 3  (1.0 if same bucket)
```

**`scope mirror --all` optimization (for large repos):**
For repos with > 500 files, full O(N²) pairwise comparison is slow. Use
Locality-Sensitive Hashing (LSH) to find approximate nearest neighbors:
```
1. Hash each GraphSignature into a MinHash signature (128 hash functions)
2. Bucket by LSH band (16 bands of 8 functions each)
3. Candidate pairs: only files sharing at least one LSH bucket
4. Verify candidates with exact Jaccard computation
This reduces O(N²) to approximately O(N * k) where k << N.
```

**Human-readable output:**
```
$ scope mirror src/services/stripe.js

Files structurally similar to src/services/stripe.js:

  src/services/paypal.js              similarity: 0.94
    Both export: 3 functions, 1 class
    Both import from: external payment SDKs
    Both called from: routes/checkout.js, services/billing.js
    → Strong candidate for shared PaymentProvider interface

  src/services/braintree.js           similarity: 0.87
    Similar structure, slightly different caller set
    → Also a candidate for the same interface

  src/integrations/razorpay.js        similarity: 0.71
    Similar exports but fewer callers
    → Possible candidate, less certain

Suggestion: These 3 files have nearly identical graph signatures.
Consider extracting a shared interface:
  scope simulate extract <symbols> --into src/services/payment_provider.js
```

---

### 16.17 `scope report` — Codebase Health Dashboard

**Purpose:** A single command that aggregates all of `scope`'s analytical capabilities into
a comprehensive health report. Gives teams a single number to track over time, and a
breakdown that shows where architectural debt is accumulating.

**CLI:**
```bash
scope report                            # full report to stdout (Markdown)
scope report --output health.md         # save as Markdown file
scope report --json                     # machine-readable summary
scope report --compare v1.2.0           # trend report vs named snapshot
scope report --sections risk,arch       # only specific sections
```

**Health score formula:**

```
score = 100.0

# Penalties (each violation subtracts points)
score -= layer_violations * 5.0      # each arch violation
score -= cycles * 10.0               # each dependency cycle
score -= unreachable_files * 2.0     # each dead file
score -= unexpected_cochange * 3.0   # each hidden coupling pair
score -= god_file_candidates * 4.0   # each oversized central file
score -= parse_errors * 0.5          # each file that failed to parse
score -= unused_exports_pct * 20.0   # 0-1 scale: % of exports that are unused

# Bonuses
score += imports_resolved_pct * 0.2  # 0-100 scale: bonus up to 20 points
                                     # (max 20 if 100% resolved)

# Clamp to [0, 100]
score = max(0.0, min(100.0, score))
```

**Report sections:**

1. **Header:** score, delta vs comparison, date, repo name.

2. **Coverage:** % of imports resolved, parse error count, language breakdown,
   resolution quality by language.

3. **Size & complexity:** file count, symbol count, avg fan-in/fan-out, max fan-in
   (flagged if > 30), God-file candidates (fan_in > 15 AND fan_out > 15).

4. **Risk hotspots:** top 10 files by risk score with scores, dependents, churn.

5. **Architectural health:** layer violation count (with details), cycle count (with
   cycle member list), avg instability, files flagged as unstable-and-central.

6. **Dead code:** unreachable file count and list, unused export count, uncovered
   source files (from test-map).

7. **Temporal coupling:** unexpected co-change pair count, most suspicious pairs.

8. **Public surface:** total public symbols, additions/removals since comparison,
   semver recommendation.

9. **Trend** (if `--compare` provided): each metric with before/after/delta and ↑/↓.

10. **Recommendations:** top 3 most impactful actions based on score breakdown.
    E.g., "Fixing 2 dependency cycles would improve score by +20."

**Markdown output format:**
```markdown
# scope Health Report
**Date:** 2025-03-08 14:22 UTC  
**Score:** 73/100 ↑+4 vs v1.2.0  

## Summary
| Metric | Value | Delta |
|--------|-------|-------|
| Layer violations | 2 | +0 |
| Dependency cycles | 1 | -1 ✓ |
| Unreachable files | 3 | +0 |
| Unexpected co-changes | 7 | -3 ✓ |
| Import resolution | 94.2% | +1.8% ✓ |

## Risk Hotspots
1. `src/auth/token.js` — score: 94 (CRITICAL) — 47 dependents, 23 commits/90d
...
```

---

### 16.18 `scope gate` — Metric-Based CI Enforcement

**Purpose:** Transform `scope report` data into actionable CI pass/fail decisions.
Prevent architectural regression on every PR by enforcing metric thresholds.

**Configuration file: `.scope/gates.toml`**

```toml
# Each [[gate]] defines one metric threshold.
# severity = "error" → failure exits with code 1
# severity = "warning" → logged but does not fail

[[gate]]
metric = "layer_violations"
max = 0
severity = "error"
message = "Layer violations introduced — fix before merging"

[[gate]]
metric = "cycles"
max = 0
severity = "error"
message = "Dependency cycles must not be introduced"

[[gate]]
metric = "max_file_fan_in"
max = 50
severity = "warning"
message = "A file has become a dependency bottleneck — consider splitting"

[[gate]]
metric = "health_score_delta"
min_delta = -5
severity = "error"
message = "Health score dropped by more than 5 points vs baseline"
# This gate uses --compare baseline, requiring a saved snapshot

[[gate]]
metric = "public_surface_removed"
max = 0
severity = "error"
message = "Public API removed — verify this is intentional (breaking change)"

[[gate]]
metric = "unreachable_files_added"
max = 0
severity = "warning"
message = "New dead code detected — remove before merging"

[[gate]]
metric = "unexpected_cochange_pairs_added"
max = 2
severity = "warning"
message = "New hidden coupling detected"

[[gate]]
metric = "imports_unresolved_pct"
max = 10
severity = "warning"
message = "More than 10% of imports are unresolved — check language adapter coverage"
```

**Available gate metrics:**
```
Absolute metrics (use 'max' threshold):
  layer_violations          — count of arch rule violations
  cycles                    — count of dependency cycles
  max_file_fan_in           — maximum fan-in of any single file
  parse_errors              — count of files that failed to parse
  unreachable_files         — count of files reachable from no entry point
  unused_exports            — count of exported symbols never imported
  unexpected_cochange_pairs — count of unexpected temporal coupling pairs
  public_surface_removed    — count of removed public symbols since comparison
  health_score              — composite score (use 'min' threshold)

Delta metrics (compare vs --compare ref, use 'min_delta' or 'max_delta'):
  health_score_delta          — change in health score
  layer_violations_delta      — change in violation count
  cycles_delta                — change in cycle count
  unreachable_files_added     — new unreachable files (not just total count)
  unexpected_cochange_pairs_added — new unexpected pairs

Percentage metrics (use 'max' or 'min' threshold):
  imports_unresolved_pct    — % of imports that are unresolved
  imports_resolved_pct      — % resolved (use 'min' threshold)
```

**`scope gate --compare <ref>` workflow:**

```bash
# Recommended CI pipeline:
# 1. On merge to main: save a snapshot
scope snapshot save --name "main-$(git rev-parse --short HEAD)"

# 2. On each PR: run gate comparing to latest main snapshot
MAIN_SNAP=$(scope snapshot list --json | jq -r '.[0].name')
scope gate --compare "${MAIN_SNAP}" --json
# Exit code 1 if any error-severity gate fails
```

**Delta gate semantics:**
Delta gates only work with `--compare`. When comparing:
- Load the comparison snapshot as baseline
- Compute metric values for baseline edge list
- Compute metric values for current index
- delta = current - baseline
- Gate: if delta < min_delta OR delta > max_delta → violation

This allows an existing codebase with known violations to pass CI — only *new*
violations fail the build. Teams fix existing debt at their own pace.

**Human-readable output:**
```
$ scope gate --compare main-abc1234

Evaluating 7 gates vs snapshot: main-abc1234

✓  layer_violations: 2 (max: 2, no regression)
✗  cycles: 1 (max: 0, introduced: 1)
   ERROR: Dependency cycles must not be introduced
   New cycle: src/auth/middleware.js → src/services/user.js → src/auth/token.js → ...
⚠  max_file_fan_in: 54 (max: 50)
   WARNING: A file has become a dependency bottleneck — consider splitting
   File: src/auth/middleware.js (fan_in: 54)
✓  health_score_delta: +2 (min_delta: -5)
✓  public_surface_removed: 0 (max: 0)
✓  unreachable_files_added: 0 (max: 0)
✓  unexpected_cochange_pairs_added: 1 (max: 2)

Result: 1 error, 1 warning. Exit code: 1
```

---

### 16.19 `scope serve` — Local Graph Query Server + Web UI

**Purpose:** Make the dependency graph visually explorable for onboarding, architecture
reviews, and ad-hoc investigation. Zero configuration, starts instantly, runs offline.

**CLI:**
```bash
scope serve                     # localhost:7777, opens browser
scope serve --port 8080         # custom port
scope serve --open              # open browser automatically
scope serve --no-ui             # API only (no web UI served)
scope serve --json              # confirm startup with JSON status
```

**HTTP API surface:**

All endpoints return `Content-Type: application/json` and mirror the CLI `--json` output.

```
GET /api/status                         — index health + last indexed timestamp
GET /api/deps?file=<path>&reverse=true&transitive=false
GET /api/symbols?file=<path>&public_only=false
GET /api/impact?target=<sym>&change_type=signature&depth=5
GET /api/callers?symbol=<qualname>&transitive=false
GET /api/calls?symbol=<qualname>&transitive=false
GET /api/why?from=<path>&to=<path>&max_paths=3
GET /api/context?target=<path>&change_type=rename&budget=8000
GET /api/risk?days=90&threshold=0&top=50
GET /api/stability?flag_threshold=0.5
GET /api/cochange?file=<path>&threshold=0.7
GET /api/entry/list
GET /api/entry/unreachable
GET /api/graph/subgraph?file=<path>&depth=2    — for web UI rendering
GET /api/report
GET /api/arch/violations
GET /api/snapshot/list
GET /api/search?q=<query>&kind=file|symbol     — search across index
```

**Axum server architecture:**

```rust
// scope-core/src/serve.rs
pub async fn start_server(port: u16, open: bool) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        db_path: find_db_path()?,
    });

    let app = Router::new()
        // API routes
        .route("/api/deps",          get(api::deps))
        .route("/api/symbols",       get(api::symbols))
        .route("/api/impact",        get(api::impact))
        .route("/api/why",           get(api::why))
        .route("/api/graph/subgraph",get(api::subgraph))
        .route("/api/report",        get(api::report))
        .route("/api/search",        get(api::search))
        // ... other routes

        // CORS for local development
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Serve embedded web UI at /
    let app = app.fallback(serve_web_ui);

    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    if open {
        let _ = open::that(format!("http://localhost:{port}"));
    }
    eprintln!("scope serve: http://localhost:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

// Web UI is a single HTML file compiled into the binary
static WEB_UI_HTML: &str = include_str!("../web_ui/dist/index.html");

async fn serve_web_ui() -> impl IntoResponse {
    Html(WEB_UI_HTML)
}
```

**Web UI architecture:**

Single-file HTML/JS application (no build step for users). Built with:
- **D3.js** (v7) for force-directed graph rendering, loaded from inline bundle
- **Vanilla JS** (no framework) for state management and DOM updates
- **CSS custom properties** for theming (dark/light mode auto-detection)
- Compiled to a single `index.html` file embedded as `include_str!` in Rust

Web UI features:
1. **File search** — instant search across indexed files and symbols (fuzzy match)
2. **Node detail panel** — click any node: see deps, reverse deps, symbols, risk score
3. **Visual impact analysis** — enter a target + change type, affected nodes highlighted
4. **Shortest path visualization** — select two nodes, path highlighted on graph
5. **Heatmap overlays:**
   - Risk score (red gradient by risk_score)
   - Instability index (blue gradient by I value)
   - Co-change rate (orange for unexpected co-changes)
   - Layer coloring (each arch layer gets a distinct color)
6. **Subgraph expansion** — start with one file, expand to N degrees of neighbors
7. **Architecture layer overlay** — nodes colored by layer, violations shown as red edges
8. **Filter panel** — hide test files, hide external imports, filter by language

**Performance notes for large repos:**
- Web UI renders only the visible subgraph (default: 2 hops from selected node)
- Full graph of 10K files would be unrenderable — always start from a single node
- `GET /api/graph/subgraph?file=<path>&depth=2` returns only the relevant subgraph
- Progressive loading: expand on click, not full graph upfront

---

### 16.20 `scope query` — Interactive Graph REPL

**Purpose:** A composable query language for exploring the dependency graph interactively.
Think `jq` for dependency graphs — powerful for ad-hoc investigation and scripting.

**CLI:**
```bash
scope query                           # interactive REPL with history
scope query --expr 'file "src/auth.js" | .deps'  # single expression (non-interactive)
```

**Language reference:**

Selection primitives:
```
file "path/to/file.js"     → select a file node by path
symbol "qualname"          → select a symbol node by qualified name
all-files                  → select all indexed file nodes
all-symbols                → select all indexed symbol nodes
```

Traversal operations (applied to a file/symbol selection):
```
.deps              → forward dependencies (what this imports)
.reverse           → reverse dependencies (what imports this)
.symbols           → symbols defined in selected files
.callers           → callers of selected symbols
.callees           → callees of selected symbols
.transitive        → make next traversal transitive (modifier)
.depth <n>         → limit next traversal to N hops (modifier)
```

Filter operations:
```
filter certainty = exact          → keep only exact-certainty results
filter certainty >= resolved      → keep exact and resolved
filter distance <= 2              → keep results within 2 hops
filter kind = function            → keep only functions
filter visibility = public        → keep only public symbols
filter file matches "src/auth/**" → keep only files matching glob
filter risk > 50                  → keep files with risk score > 50 (requires risk data)
```

Set operations:
```
intersect <expr>    → keep results present in both current set and expr result
union <expr>        → merge current set with expr result
minus <expr>        → remove results present in expr from current set
```

Aggregations:
```
count              → return count of results as a number
unique             → deduplicate (by file path or qualname)
sort-by risk       → sort by risk score
sort-by distance   → sort by traversal distance (default)
```

Variable binding:
```
let <name> = <expr>    → bind result to a name for later use
$<name>                → reference a previously bound variable
```

Meta commands (REPL only):
```
:help              → show command reference
:history           → show command history
:save-query "<n>"  → save current expression to .scope/queries.toml
:load-query "<n>"  → load a saved query expression
:exit              → exit REPL
```

**Example session:**
```
scope> let auth = file "src/auth/middleware.js"

scope> $auth | .reverse | count
18

scope> $auth | .deps | filter certainty = exact
3 files:
  src/utils/jwt.js        (exact)
  src/models/user.js      (exact)
  src/config/constants.js (exact)

scope> let risky = all-files | filter risk > 60
scope> $auth | .deps | intersect $risky
2 files:
  src/utils/jwt.js        risk: 71
  src/models/user.js      risk: 67

scope> $auth | .transitive .depth 3 .reverse | unique | count
47

scope> :save-query "auth-high-risk-deps"
Saved to .scope/queries.toml

scope> all-files | filter risk > 80 | .reverse | .depth 1 | unique | count
# How many files depend on CRITICAL-risk files?
234
```

**Non-interactive scripting:**
```bash
# Count direct callers of all critical-risk files
scope query --expr 'all-files | filter risk > 80 | .reverse | .depth 1 | unique | count'

# Find all public functions reachable from cli entry point
scope query --expr 'file "src/cli.js" | .transitive .deps | .symbols | filter visibility = public | filter kind = function'

# Find files with unexpected co-changes that are also high-risk
scope query --expr 'cochange --unexpected | intersect (all-files | filter risk > 50)'
```

**Implementation architecture:**

```rust
// scope-core/src/query_lang.rs

// 1. Lexer: tokenize input string → Vec<Token>
// 2. Parser: recursive descent → QueryExpr AST
// 3. Evaluator: QueryExpr → QueryResult (file set | symbol set | number)

pub enum QueryExpr {
    // Primitives
    FileSelect(String),
    SymbolSelect(String),
    AllFiles,
    AllSymbols,
    // Traversal
    Pipe(Box<QueryExpr>, Box<QueryExpr>),
    Deps(Modifiers),
    Reverse(Modifiers),
    Symbols,
    Callers(Modifiers),
    Callees(Modifiers),
    // Filters
    Filter(FilterExpr),
    // Set ops
    Intersect(Box<QueryExpr>),
    Union(Box<QueryExpr>),
    Minus(Box<QueryExpr>),
    // Aggregations
    Count,
    Unique,
    SortBy(SortKey),
    // Variables
    Let(String, Box<QueryExpr>),
    Var(String),
}

pub struct Modifiers {
    pub transitive: bool,
    pub depth: Option<u32>,
}

pub enum QueryResult {
    Files(Vec<FileResult>),
    Symbols(Vec<SymbolResult>),
    Number(u64),
    Error(String),
}

// Evaluator dispatches each QueryExpr to the appropriate scope-core query function
pub fn evaluate(expr: &QueryExpr, ctx: &mut QueryContext) -> QueryResult {
    match expr {
        QueryExpr::FileSelect(path) => { /* DB lookup */ }
        QueryExpr::Deps(mods) => { /* query::deps() */ }
        QueryExpr::Intersect(rhs) => { /* set intersection */ }
        // ...
    }
}
```

**REPL implementation:**
```rust
// Uses rustyline for readline support, history, tab completion
let mut rl = rustyline::DefaultEditor::new()?;
let history_path = scope_dir.join("query_history.txt");
let _ = rl.load_history(&history_path);

loop {
    let readline = rl.readline("scope> ");
    match readline {
        Ok(line) => {
            rl.add_history_entry(&line)?;
            let result = evaluate_line(&line, &mut ctx);
            print_result(&result);
        }
        Err(ReadlineError::Interrupted) => break,  // Ctrl-C
        Err(ReadlineError::Eof) => break,           // Ctrl-D
        Err(e) => eprintln!("Error: {e}"),
    }
}

rl.save_history(&history_path)?;
```

**Tab completion:**
- After `file "` → complete with indexed file paths
- After `symbol "` → complete with indexed qualnames
- Top-level keywords: `file`, `symbol`, `all-files`, `let`, `:help`, etc.
- After `filter` → complete with filter field names


---

## 17. Phases and milestones

### Phase 0 — Node.js POC (~1 day)

**Goal:** Validate graph extraction accuracy and query correctness in a fast-iteration
environment before committing to the full Rust implementation. Node.js allows quick
experimentation with tree-sitter and SQLite without compile times.

**Why this matters:** The most important risk in this project is extraction quality —
do tree-sitter parsers give us accurate enough data? Proving this in Node.js costs one
day. Getting it wrong in Rust costs weeks of rework.

**Deliverables:**
- `poc/index.js` — Node.js script using `node-tree-sitter`, `better-sqlite3`
- Extracts imports, exports, function definitions, call sites from JS/TS source files
- Builds SQLite edge list with certainty labels
- Implements `deps`, `callers`, `impact` (BFS traversal), `why` (Dijkstra shortest path)
- JSON output for all queries
- Tested against at least one real open-source project (e.g., Express, Fastify)

**Acceptance criteria:**
- `scope impact src/router.js` on Express correctly identifies all files affected by
  a signature change in the router module.
- `scope why src/utils/merge.js src/application.js` returns a valid path showing the
  import chain.
- Parse error rate < 5% on a real project (dynamic imports labeled `dynamic`, not errors).

**Tasks:**
- [ ] `npm init`, install `node-tree-sitter`, `better-sqlite3`, `ignore`
- [ ] Implement file walker respecting `.gitignore`
- [ ] tree-sitter JS grammar: extract import statements, export statements, function defs
- [ ] tree-sitter JS grammar: extract call sites (function calls, method calls)
- [ ] SQLite schema: files, symbols, imports, file_edges, symbol_edges
- [ ] Import resolver: resolve relative paths to absolute file paths
- [ ] Implement BFS traversal for `deps`, `impact`
- [ ] Implement Dijkstra for `why`
- [ ] Test on `express` source (clone, run POC, verify output)
- [ ] Document findings in `POC.md`: what worked, what edge cases were found

---

### Milestone 0 — Foundation (~1 day)

**Goal:** Rust project skeleton with all architecture decisions locked in. After this
milestone, all subsequent milestones add features to a stable foundation without
refactoring the core.

**Deliverables:**
- Cargo workspace with `scope-core`, `scope-cli`, `scope-mcp` stubs
- Config loading: repo root detection, `.scope/` directory management
- SQLite migration system (run pending migrations on DB open)
- Logging with `tracing` (RUST_LOG=debug for verbose, info by default)
- Base error types with `thiserror`
- JSON response envelope type with `schema_version`
- CLI argument stubs: all commands parsed, all return empty valid JSON
- `scope --help` and `scope <cmd> --help` work for all commands
- Test fixture repos created (empty for now, structure only)

**Tasks:**
- [ ] `cargo new --lib crates/scope-core`, `cargo new --bin crates/scope-cli`
- [ ] Root `Cargo.toml` with all workspace dependencies pinned
- [ ] `config.rs`: `find_repo_root()`, `ensure_scope_dir()`, `Config` struct
- [ ] `store.rs`: `open_db()`, `run_migrations()`, `MIGRATIONS` const array, `index_meta`
- [ ] `output.rs`: `JsonEnvelope<T>` with `schema_version`, human formatter trait
- [ ] Error enum: `ScopeError` variants for all expected failure modes
- [ ] `main.rs`: clap `Commands` enum with all subcommands, dispatch stubs
- [ ] `tracing_subscriber` initialization from `RUST_LOG` env var
- [ ] Fixture directories created: `fixtures/rust_small/`, `fixtures/ts_small/`,
      `fixtures/dynamic_limits/`, `fixtures/arch_violations/`
- [ ] Migration `001_initial.sql` with complete schema from §13

**Fixture content for `rust_small/`:**
```
src/
  main.rs           — uses lib.rs, calls greet()
  lib.rs            — pub fn greet(), pub fn farewell(), uses parser.rs
  parser.rs         — pub fn parse(), internal fn tokenize()
  resolver.rs       — pub fn resolve(), calls parser::parse()
  utils.rs          — pub fn format_output(), no dependencies
```

**Fixture content for `ts_small/`:**
```
src/
  index.ts          — imports from auth/, utils/
  auth/
    index.ts        — barrel: re-exports from middleware.ts
    middleware.ts   — export function verifyToken(), calls jwt.ts
    jwt.ts          — export function sign(), export function verify()
  utils/
    logger.ts       — export function log()
    formatter.ts    — export function format(), imports logger.ts
```

---

### Milestone 1 — File dependency graph (~1–2 days)

**Goal:** Answer file import and reverse-import questions reliably for Rust source files.
The foundation of all other features — every subsequent feature builds on file edges.

**Deliverables:**
- Repo scanner using `ignore` crate (respects `.gitignore`, `.scopeignore`)
- Rust language adapter: import extraction only (not symbols yet)
- File nodes and file edges persisted to SQLite
- `scope index` command (full, non-incremental)
- `scope deps <file>` — forward dependencies
- `scope deps <file> --reverse` — reverse dependencies
- `scope deps <file> --transitive` — full transitive closure
- Golden JSON tests for all three modes on `rust_small` fixture

**Acceptance criteria:**
- `scope deps src/lib.rs` on `rust_small` returns exactly `[src/parser.rs]`
- `scope deps src/parser.rs --reverse` returns `[src/lib.rs, src/resolver.rs]`
- `scope deps src/main.rs --transitive` returns all reachable files
- JSON output matches golden files exactly
- Parse error rate: 0% on `rust_small`, < 5% on `dynamic_limits`

**Tasks:**
- [ ] `scanner.rs`: `walk_repo(root, config) → Vec<(PathBuf, Metadata)>` using `ignore` crate
- [ ] `adapters/mod.rs`: `Adapter` trait definition + `ExtractResult`, `ImportRecord`, `Span`
- [ ] `adapters/rust.rs`: tree-sitter Rust grammar, extract `use` declarations
      Handle: `use crate::x`, `use super::x`, `use self::x`, `pub use ...`
      Resolve: `crate::parser` → `src/parser.rs` (try `.rs` and `/mod.rs`)
- [ ] `store.rs`: `upsert_file()`, `insert_file_edge()`, `query_deps()`, `query_reverse_deps()`
- [ ] `graph.rs`: load `DiGraph<FileId, EdgeData>` from SQLite, BFS traversal
- [ ] `query.rs`: `deps_query(file, direction, transitive, depth)` function
- [ ] CLI: `scope index` calls indexing pipeline, `scope deps` dispatches to query
- [ ] Golden JSON files in `tests/golden/deps_basic.json`, `deps_reverse.json`, `deps_transitive.json`
- [ ] Integration test: full round-trip on `rust_small` fixture

**Rust import patterns to handle:**
```rust
use crate::parser;                    // → src/parser.rs
use crate::parser::parse_file;        // → src/parser.rs (symbol reference)
use crate::utils::{format, display};  // → src/utils.rs or src/utils/mod.rs
use super::resolver;                  // → sibling module
mod embedded;                         // → src/embedded.rs or src/embedded/mod.rs
pub use crate::types::Result;         // → import + re-export
use std::collections::HashMap;        // → external (skip for file graph)
```

---

### Milestone 2 — Symbol inventory (~1 day)

**Goal:** Know what every file exposes — names, kinds, visibility, and byte spans.
Enables `scope symbols`, lays groundwork for call graph and impact analysis.

**Deliverables:**
- Symbol extraction added to Rust adapter (on top of import extraction from M1)
- Symbols table populated on index
- `scope symbols <file>` command
- `scope symbols <file> --public-only`
- Golden JSON tests for `rust_small` and `rust_medium` fixtures

**Acceptance criteria:**
- `scope symbols src/lib.rs` on `rust_small` returns `greet` (public), `farewell` (public),
  and any private helpers with correct visibility labels
- All `span_start` / `span_end` values correct to within 5 bytes (tree-sitter accuracy)
- Exported symbols flagged with `exported: true` for `pub` items at crate root

**Tasks:**
- [ ] `adapters/rust.rs`: add symbol extraction
      Extract: `fn`, `pub fn`, `struct`, `pub struct`, `enum`, `pub enum`, `type`, `const`,
      `static`, `trait`, `pub trait`, `impl` blocks (collect method names)
- [ ] Visibility mapping: `pub` → Public, `pub(crate)` → Package, `pub(super)` → Module,
      no modifier → Local (for fn inside impl), Module (for fn at module root)
- [ ] Qualname construction: `crate::` prefix + module path from file path
- [ ] `store.rs`: `insert_symbol()`, `query_symbols(file_id, filters)`
- [ ] CLI: `scope symbols` command with `--public-only`, `--kind` flags
- [ ] `rust_medium` fixture: nested modules, re-exports, trait impls
- [ ] Golden JSON tests
- [ ] Unit tests for qualname construction, visibility mapping

**Symbol kinds and tree-sitter node types:**
```
function_item      → Function
impl_item          → (collect methods inside as Method)
struct_item        → Struct
enum_item          → Enum
type_alias         → TypeAlias
trait_item         → Trait
const_item         → Constant
static_item        → Static
mod_item           → Module
```

---

### Milestone 3 — Direct call graph (~1–2 days)

**Goal:** Answer "who calls this function?" and "what does this function call?" for
resolvable in-repo symbols. Correctness here directly affects impact analysis quality.

**Deliverables:**
- Call site extraction added to Rust adapter
- Symbol edges (kind=`call`) populated on index
- `scope calls <symbol>` — direct callees
- `scope callers <symbol>` — direct callers
- `scope callers <symbol> --transitive` — transitive callers
- Golden JSON tests

**Acceptance criteria:**
- `scope callers crate::parser::parse_file` on `rust_small` returns `crate::resolver::resolve`
- Unresolved calls labeled `heuristic`, never promoted to `resolved`
- Macro calls extracted but labeled `heuristic` (macros can expand to anything)
- Method calls via trait objects labeled `heuristic` (dynamic dispatch)

**Tasks:**
- [ ] `adapters/rust.rs`: call site extraction
      tree-sitter node types: `call_expression`, `method_call_expression`
      Extract callee: field `function` of `call_expression` → path expression → qualname
- [ ] Symbol resolution: for each call site, attempt to resolve callee_name to a symbol
      Same-file resolution: check symbols in same file_id
      Imported symbol resolution: check `imports` table for matching qualname prefix
      Unresolved: certainty=heuristic
- [ ] `store.rs`: `insert_symbol_edge()`, `query_callers()`, `query_callees()`
- [ ] `graph.rs`: BFS on symbol_graph for transitive callers/callees
- [ ] CLI: `scope calls`, `scope callers` commands
- [ ] Golden JSON for `rust_small` (simple call chains) and `rust_medium` (cross-module)
- [ ] Test: `dynamic_limits` fixture — verify all unresolvable calls are `heuristic`

**Edge cases in Rust call resolution:**
```rust
// Direct call — exact
resolver::resolve_file(path)

// Method call on known type — resolved
let r = ImportResolver::new();
r.resolve(import)

// Closure call — heuristic (target unknown)
let handler = get_handler();
handler()

// Trait method via generic — heuristic
fn process<T: Parser>(p: T) { p.parse() }

// Macro call — heuristic (expands to arbitrary code)
vec![a, b, c]
println!("{}", value)
```

---

### Milestone 4 — Impact engine (~1–2 days)

**Goal:** The core product value. Given a target and change type, compute the blast radius
with explanations. This milestone unlocks the primary agent use case.

**Deliverables:**
- `scope impact <target> --change-type <type>` for all 6 change types
- Reason trails on every impacted node
- Grouped output: high-confidence vs uncertain
- `scope explain <target>` — full evidence trail for a node
- Golden JSON tests for all 6 change types on both fixtures

**Acceptance criteria:**
- `scope impact crate::resolver::resolve_symbol --change-type signature` includes all files
  that call `resolve_symbol` (direct and transitive)
- `scope impact crate::resolver::resolve_symbol --change-type body` includes only direct
  callers (not transitive)
- `scope impact src/parser.rs --change-type delete` includes all files that import
  `parser.rs` (since they will have broken import statements)
- Every result node has non-empty `reason` string and correct `distance`

**Tasks:**
- [ ] `query.rs`: `impact_query(target, change_type, depth_limit)` with traversal rules
      per change type (see §12 for rules)
- [ ] Reason string generation: template-based per edge kind
      `call`: "calls {target} directly"
      `import`: "imports {file} which contains target"
      `re-export`: "re-exports {symbol} from {file}"
- [ ] Distance tracking through BFS traversal
- [ ] Certainty propagation: min(path_certainties)
- [ ] `scope explain <target>`: show all edges incident to a node with certainties
- [ ] CLI: `scope impact --change-type` dispatch, output grouping, `scope explain`
- [ ] Golden JSON: `impact_body.json`, `impact_signature.json`, `impact_rename.json`,
      `impact_delete.json`, `impact_visibility.json`, `impact_side_effect.json`

---

### Milestone 5 — Path and context queries (~1 day)

**Goal:** Answer "why are these files connected?" and "what is the minimum I need to read?"
These are the two highest-value features for agent workflows.

**Deliverables:**
- `scope why <a> <b>` with Dijkstra shortest path + Yen's k-shortest
- `scope context <task>` with BFS + scoring + `must_read`/`should_read`/`skip` classification
- `scope pack <target> --budget <tokens>` with token-budget-aware formatting

**Acceptance criteria:**
- `scope why src/utils.rs src/main.rs` returns correct path with hop-level reasons
- `scope context --target src/auth/middleware.js --change-type rename` includes the 5
  expected must-read files from `ts_small` fixture
- `scope pack --budget 2000` output is under 2000 tokens (verified with tiktoken-rs)
- `scope why` returns "no path found" for disconnected nodes, not a crash

**Tasks:**
- [ ] `graph.rs`: `dijkstra_shortest_path(from, to, weight_fn)` using petgraph
- [ ] `graph.rs`: `yen_k_shortest_paths(from, to, k)` — Yen's algorithm over petgraph
- [ ] `query.rs`: `why_query(from, to, max_paths)` with hop annotation
- [ ] `query.rs`: `context_query(targets, change_type, budget)` with BFS + scoring
- [ ] `context_pack.rs`: `pack_format(context_result, budget)` with tiktoken-rs
      token counting, section-by-section budget enforcement
- [ ] CLI: `scope why`, `scope context`, `scope pack` commands
- [ ] Golden JSON: `why_basic.json`, `context_rename.json`
- [ ] Test: `scope why` with disconnected nodes returns correct error

---

### Milestone 6 — Incremental indexing (~1 day)

**Goal:** Re-indexing cheap enough to run on every file save without disrupting flow.
Single-file edits should take <100ms to re-index on a normal laptop.

**Deliverables:**
- blake3 content hash stored per file
- Changed-file detection: only re-index files whose hash has changed
- Deleted-file cleanup via CASCADE DELETE
- `scope index` is now always incremental by default (full index only on first run
  or with `--full` flag)
- Benchmark showing speedup: single-file change on `rust_medium` fixture

**Acceptance criteria:**
- Unchanged files produce zero DB writes
- Single-file edit on 30-file fixture: re-index in <50ms
- Deleted file: all its edges and symbols removed from DB (verified by `scope deps`)
- New file that was previously unresolved: its dependent files' imports now resolve

**Tasks:**
- [ ] `scanner.rs`: hash each file content during scan
- [ ] `store.rs`: `files_with_hashes() → HashMap<PathBuf, String>` (current DB state)
- [ ] `indexer.rs` (new): orchestrate changed/new/deleted partition
- [ ] Verify: after re-index, all golden JSON tests still pass
- [ ] `benches/incremental_bench.rs`: measure full vs incremental on `rust_medium`
- [ ] `scope doctor`: add check — report % of imports unresolved, parse error files

---

### Milestone 7 — TypeScript/JavaScript adapter (~1–2 days)

**Goal:** Unlock the second most important language family for agent workflows.
JS/TS has significantly more complex module semantics than Rust; this milestone requires
careful handling of barrel files, CommonJS, and dynamic imports.

**Deliverables:**
- `adapters/typescript.rs` and `adapters/javascript.rs` (shared logic, different grammars)
- Import extraction: ES modules (static and dynamic), CommonJS `require()`
- Symbol extraction: `function`, `class`, `const/let/var` at top level, `export default`
- Barrel file detection and re-export tracing
- TypeScript-specific: `interface`, `type` alias, `namespace`
- `ts_small` fixture: all golden tests passing
- `dynamic_limits` fixture: all dynamic imports labeled `dynamic`

**Tasks:**
- [ ] `adapters/javascript.rs`: tree-sitter-javascript grammar
      Import: `import { } from`, `import * as`, `import()`, `require()`
      Export: `export function`, `export const`, `export default`, `module.exports`
      Calls: `call_expression` nodes
- [ ] `adapters/typescript.rs`: extends javascript with:
      `interface_declaration`, `type_alias_declaration`, `namespace_declaration`
      `tsconfig.json` path resolution (load baseUrl and paths mappings)
- [ ] Barrel file detection: flag files where ≥50% of statements are re-exports
- [ ] Resolution order for `./foo`: try `.ts`, `.tsx`, `.js`, `/index.ts`, `/index.js`
- [ ] CommonJS: `require('./foo')` → Relative if string literal, Dynamic if variable
- [ ] `dynamic_limits` fixture: `require(variable)`, `import(computedPath)` → `dynamic`
- [ ] Golden JSON tests for `ts_small` (all commands)
- [ ] Integration test: `scope index` on a real TypeScript project

---

### Milestone 8 — Architectural analysis (~1–2 days)

**Goal:** Transform `scope` from a query tool into an enforcement and insight tool.
Enables CI integration for the first time.

**Deliverables:**
- `scope arch check` with `.scope/arch.toml` layer/rule engine
- `scope arch init` for auto-detected starter config
- `scope arch explain <file>`
- `scope stability` — Martin instability index
- `scope risk` — churn-weighted blast radius with git log integration
- `arch_violations` fixture with golden tests

**Acceptance criteria:**
- `scope arch check` on `arch_violations` fixture detects all 3 deliberate violations
- `scope arch check` exits with code 1 when violations found, code 0 when none
- `scope stability` on `rust_small`: constants.js I=0.02, leaf files I≈1.0
- `scope risk` on any fixture with git history: scores computed correctly
- `scope arch init` on `ts_small` generates a valid arch.toml

**Tasks:**
- [ ] `arch.rs`: parse `.scope/arch.toml` → `ArchConfig`, pattern matching via `glob` crate
- [ ] `arch.rs`: `check_violations(graph, arch_config) → Vec<Violation>`
- [ ] `arch.rs`: `explain_file(file, arch_config) → FileArchExplanation`
- [ ] `arch.rs`: `arch_init(repo_root) → ArchConfig` (directory name detection)
- [ ] `stability.rs`: `compute_stability(file_graph) → Vec<StabilityResult>` (SQL queries)
- [ ] `risk.rs`: `populate_git_churn(db, window_days)` — run `git log --name-only`
      Parse output: `commit_sha\n\nfile1\nfile2\n\n` format
      Store in `file_churn` table
- [ ] `risk.rs`: `compute_risk(db, graph, window_days) → Vec<RiskResult>`
- [ ] CLI: `scope arch check`, `scope arch explain`, `scope arch init`, `scope stability`, `scope risk`
- [ ] `arch_violations` fixture with `.scope/arch.toml` and 3 deliberate violations
- [ ] Golden JSON: `arch_violations.json`, `stability_basic.json`, `risk_basic.json`
- [ ] Test: `scope arch check` exit codes (0 for clean, 1 for violations)

---

### Milestone 9 — Surface, rename plan, test map (~1–2 days)

**Goal:** Higher-order features that enable safe refactoring and API management.

**Deliverables:**
- `scope surface` — public API surface extraction
- `scope surface diff <ref1> <ref2>` — semver diff between refs
- `scope rename-plan <old> <new>` — dry-run topological rename plan
- `scope rename-plan --apply` — safe byte-offset file rewriter
- `scope test-map build/covers/covered-by/uncovered`
- `rename_fixtures` fixture with golden tests

**Acceptance criteria:**
- `scope surface diff v1 v2` on fixture with known symbol additions/removals: exact match
- `scope rename-plan verifyToken validateToken` on `rename_fixtures`: correct topological
  order with all 14 expected sites
- `scope rename-plan --apply` correctly rewrites all 4 files in `rename_fixtures`
- `scope test-map covers src/auth/middleware.js` returns all 3 expected test files

**Tasks:**
- [ ] `surface.rs`: `extract_surface(db) → Vec<PublicSymbol>`, `diff_surfaces(A, B)`
- [ ] `rename_plan.rs`: collect all site types, topological sort, substitution list
- [ ] `rename_plan.rs`: `apply_rename_plan(plan, dry_run)` — atomic file rewriter
      Write temp file, verify, rename into place
      Sort substitutions by start_byte descending before applying
- [ ] `test_map.rs`: `detect_test_files(db, patterns)`, `build_coverage_map(graph, tests)`
- [ ] CLI: `scope surface`, `scope surface diff`, `scope rename-plan`, `scope test-map`
- [ ] `rename_fixtures` fixture with 14 known sites
- [ ] Test: `--apply` correctly transforms all files, index still valid after rename

---

### Milestone 10 — Snapshots and agent integration (~1 day)

**Goal:** Architectural time travel and polished agent workflow. First release candidate.

**Deliverables:**
- `scope snapshot save/list/delete` — graph snapshot management
- `scope diff-snapshot <snap1> <snap2>` — architectural diff
- Finalized JSON contracts with stable `schema_version`
- `scope-mcp` stdio wrapper (thin layer over `scope-core`)
- `CLAUDE.md` integration snippet
- `cargo install scope-cli` works

**Acceptance criteria:**
- Snapshot save/load round-trips without data loss
- `scope diff-snapshot` correctly detects new cycles and violations
- MCP wrapper exposes all core commands as MCP tools
- `CLAUDE.md` tested with Claude Code agent

**Tasks:**
- [ ] `snapshot.rs`: `save_snapshot(name, git_ref, db) → Snapshot`
      Serialize: edge list to JSON, compress with zstd
      Store in `snapshots` table
- [ ] `snapshot.rs`: `load_snapshot(name, db) → SnapshotData`
      Decompress + deserialize
- [ ] `snapshot.rs`: `diff_snapshots(A, B, arch_config) → SnapshotDiff`
      Edge set diff, cycle re-detection, arch rule re-evaluation, stability delta
- [ ] CLI: `scope snapshot save/list/delete`, `scope diff-snapshot`
- [ ] `scope-mcp/src/main.rs`: stdio MCP server
      Each tool: parse MCP tool_call JSON, dispatch to `scope-core`, return MCP result
      Tools: deps, symbols, impact, why, context, risk, arch_check, stability, cochange
- [ ] `CLAUDE.md`: write integration guide with all recommended pre/post-edit commands
- [ ] `README.md`: `cargo install scope-cli`, quick start guide
- [ ] CI: publish crate to crates.io on git tag

---

### Milestone 11 — Extra utilities and Polish (~1 day)

**Goal:** Complete the standard graph utility commands and polish the CLI experience.

**Deliverables:**
- `scope unused` — dead exports (exported but never imported)
- `scope cycles` — circular dependency chains with severity
- `scope diff <branch>` — blast radius of git branch diff
- `scope tree <path> --depth N` — recursive dependency tree (visual)
- `scope doctor` — comprehensive index health report
- `scope benchmark` — full timing breakdown
- Multi-language integration test suite (Rust + TS on same repo)

**Tasks:**
- [ ] `query.rs`: `unused_exports(db)` — symbols WHERE exported=1 AND no incoming symbol_edges
- [ ] `graph.rs`: `find_cycles()` — Tarjan's SCC; severity = cycle length (longer = worse)
- [ ] `query.rs`: `diff_branch(branch)` — git diff `main..branch` → changed files → impact
- [ ] `output.rs`: `tree_format(nodes, depth)` — ASCII tree rendering
- [ ] `query.rs`: `doctor(db, graph) → DoctorReport` — coverage stats, error files, index age
- [ ] CLI: all above commands
- [ ] `benches/`: full benchmark suite across all fixture repos

---

### Milestone 12 — Temporal coupling and simulation (~2 days)

**Goal:** The first "beyond static analysis" features — temporal coupling from git history
and in-memory graph simulation for refactoring validation.

**Deliverables:**
- `scope cochange` — full temporal coupling detection with co-change matrix
- `scope simulate extract` — in-memory graph mutation with stability projections
- `cochange` fixture with prepared git history

**Tasks:**
- [ ] `cochange.rs`: `build_cochange_matrix(db, window_days)`
      SQL: build commit→files map from `file_churn`
      Compute co-occurrence counts in Rust (SQL GROUP BY is too slow for matrix)
      INSERT INTO `file_cochange`
- [ ] `cochange.rs`: `query_cochange(file, threshold, window_days)`
      Join with `file_edges` to classify expected/unexpected
- [ ] `simulate.rs`: `simulate_extract(symbols, from_file, into_file, graph, db)`
      In-memory graph clone (petgraph graph clone)
      Apply hypothetical mutations
      Cycle detection, stability recompute, arch violation check
      Return SimulateResult without any DB writes
- [ ] Cochange fixture: `fixtures/cochange/create_git_history.sh`
      Creates a test git repo with 50 commits, known co-change pairs
- [ ] CLI: `scope cochange`, `scope simulate extract`
- [ ] Golden JSON: `cochange_basic.json`, `simulate_extract.json`
- [ ] Tests: matrix computation correctness, simulation stability delta correctness

---

### Milestone 13 — Entry points, audit, and decomposition (~2 days)

**Goal:** Reachability analysis, security capability auditing, and god-file decomposition.

**Deliverables:**
- `scope entry list/cone/reaches/unreachable`
- `scope audit --capability`
- `scope split <file>` with symbol clustering
- `scope mirror <file>` with graph-signature similarity
- `dead_code` and `capability_audit` and `god_file` fixtures

**Tasks:**
- [ ] `entry.rs`: `find_entry_points(db, arch_config)`, `reachability_cone(entry, graph)`
- [ ] `entry.rs`: `unreachable_files(all_files, entry_points, graph)`
- [ ] `audit.rs`: `load_capabilities(db, arch_config)`, `capability_reach(cap, graph, db)`
      Reverse BFS from capability sources, cross-reference with entry points
- [ ] `split.rs`: `symbol_caller_profiles(file, db)`, jaccard similarity matrix,
      greedy agglomerative clustering, module name suggestion
- [ ] `mirror.rs`: `graph_signature(file, db)`, `pairwise_similarity(files)`
- [ ] Fixtures: `dead_code/`, `capability_audit/`, `god_file/`, `similarity_pairs/`
- [ ] CLI: `scope entry`, `scope audit`, `scope split`, `scope mirror`
- [ ] Golden JSON for all new commands

---

### Milestone 14 — Report, gates, serve, and REPL (~2 days)

**Goal:** Complete the platform tier: health reporting, CI gates, web UI, and REPL.
The final milestone before v1.0 release.

**Deliverables:**
- `scope report` — full health dashboard with health score
- `scope gate` — CI metric enforcement
- `scope serve` — local HTTP server + embedded web UI
- `scope query` — composable REPL with query language

**Tasks:**
- [ ] `report.rs`: `compute_health_report(db, graph, compare_snapshot)` — aggregates all metrics
      Health score formula as specified in §16.17
- [ ] `report.rs`: `render_markdown(report)` — full Markdown report output
- [ ] `gate.rs`: `load_gates_toml(.scope/gates.toml)`, `evaluate_gates(gates, report)`
      Delta gate evaluation against comparison snapshot
      Proper exit code handling
- [ ] `serve.rs`: Axum router with all API endpoints, `include_str!` web UI
- [ ] Web UI: single-file HTML/D3 force graph (see §16.19 for feature list)
      Build script: `npm run build` in `crates/scope-core/web_ui/` → `dist/index.html`
      Embedded via `include_str!("../web_ui/dist/index.html")` in serve.rs
- [ ] `query_lang.rs`: lexer, recursive descent parser, AST, evaluator (see §16.20)
      rustyline REPL with history and tab completion
- [ ] Non-interactive `--expr` mode
- [ ] CLI: `scope report`, `scope gate`, `scope serve`, `scope query`
- [ ] Integration tests: `tests/integration/serve_api.rs` — start server, hit all endpoints
- [ ] Golden JSON: `report_basic.json`, `gate_violations.json`
- [ ] Gates: test exit code 0 (pass) and 1 (fail) scenarios

---

## 18. Testing strategy

### Test categories and when to write each

**Unit tests** (`#[test]` in the module file):
Write for every non-trivial pure function. Especially:
- Path normalization in `scanner.rs`
- Visibility mapping in adapters
- Change-type traversal rule selection in `query.rs`
- Qualname construction in adapters
- Stability formula computation in `stability.rs`
- Risk score formula in `risk.rs`
- Arch rule pattern matching in `arch.rs`
- Token counting vs budget in `context_pack.rs`
- Byte-offset substitution ordering in `rename_plan.rs`
- Health score formula in `report.rs`
- Gate threshold evaluation in `gate.rs`
- Query language lexer/parser correctness in `query_lang.rs`

**Integration tests** (`tests/` directory, use fixture repos):
Write for every command that reads and writes the DB:
- Full index pipeline on each fixture repo
- Incremental index: change one file, verify only it re-indexed
- `scope deps` output matches expected graph structure
- `scope impact` returns correct blast radius for each change type
- `scope arch check` detects all violations in `arch_violations/` fixture
- `scope rename-plan --apply` correctly transforms `rename_fixtures/`
- `scope cochange` on `cochange/` fixture matches prepared co-change pairs
- `scope gate` exits 0/1 correctly

**Golden JSON tests** (`tests/golden/`):
Capture the exact JSON output of every command on fixture repos. These act as
regression tests for output format changes.

Process:
1. First run: generate golden files with `scope <cmd> --json > tests/golden/<cmd>.json`
2. Subsequent runs: compare actual output to golden file byte-for-byte
3. To update intentionally: delete golden file, regenerate, commit

Golden files must be committed. Any schema change that breaks golden files requires
a `schema_version` bump.

**Performance benchmarks** (`benches/`):
Criterion-based benchmarks for:
- Full index on `rust_medium` (target: < 1 second)
- Full index on `ts_small` (target: < 500ms)
- Incremental index, single file changed (target: < 50ms)
- `scope deps --transitive` on `rust_medium` (target: < 10ms)
- `scope impact --change-type signature` (target: < 50ms)
- `scope why` shortest path (target: < 20ms)
- `scope risk` on `rust_medium` (target: < 100ms)

### Fixture design principles

Each fixture serves a specific testing contract:

| Fixture | Primary purpose | Notable features |
|---------|-----------------|------------------|
| `rust_small/` | Baseline Rust correctness | 5 files, simple import/call graph |
| `rust_medium/` | Complex Rust patterns | Nested modules, re-exports, trait impls, generics |
| `ts_small/` | TypeScript correctness | Barrel files, ES modules, re-exports |
| `dynamic_limits/` | Failure modes | Dynamic requires, computed imports, `dynamic` certainty |
| `arch_violations/` | Arch enforcement | 3 deliberate violations, arch.toml included |
| `rename_fixtures/` | Rename correctness | 14 known sites across 4 files |
| `god_file/` | Split algorithm | 67 exports in 3 natural clusters |
| `dead_code/` | Reachability | 2 known unreachable files, 3 entry points |
| `capability_audit/` | Audit algorithm | 2 expected + 2 unexpected network-reaching entries |
| `similarity_pairs/` | Mirror algorithm | 2 payment services with 94% graph similarity |
| `cochange/` | Temporal coupling | 50-commit git history with 3 known co-change pairs |

### Reliability rules

1. Any unsupported construct must fail soft — produce partial results with
   `parse_status = "partial"`, never crash.
2. All parse errors surfaced in `scope doctor` and `scope report`.
3. Golden JSON tests must pass on every CI run.
4. No test should depend on git history of the `scope` repo itself (use
   self-contained fixture git repos created by shell scripts).
5. Tests must be deterministic — no timestamps in golden files (use placeholders).
6. Integration tests must clean up: use `tempfile` crate for DB files.

---

## 19. Performance targets

These are engineering goals, not marketing claims. Measure before shipping.

| Command | Warm query target | Notes |
|---------|-------------------|-------|
| `scope deps` | < 5ms | Pure SQL query |
| `scope symbols` | < 5ms | Pure SQL query |
| `scope callers` / `scope calls` | < 10ms | Small graph BFS |
| `scope impact` (direct only) | < 20ms | BFS, D=1 |
| `scope impact` (transitive) | < 50ms | Full BFS on file graph |
| `scope why` | < 30ms | Dijkstra on loaded graph |
| `scope context` | < 100ms | BFS + scoring |
| `scope pack` | < 200ms | Includes tiktoken-rs |
| `scope risk` | < 200ms | Requires git churn data loaded |
| `scope stability` | < 50ms | Two SQL queries |
| `scope arch check` | < 100ms | O(edges) pattern matching |
| `scope cochange` query | < 50ms | SQL lookup from precomputed table |
| `scope report` | < 2 seconds | Aggregates many queries |
| `scope gate` | < 3 seconds | Includes report + comparison |
| `scope simulate extract` | < 200ms | In-memory graph clone + mutation |
| `scope entry unreachable` | < 500ms | Full BFS from all entry points |
| Full index (1000-file repo) | < 5 seconds | rayon parallel parsing |
| Full index (10000-file repo) | < 60 seconds | Acceptable for first-run |
| Incremental index (1 file changed) | < 100ms | Always fast |
| `scope serve` startup | < 1 second | DB load + Axum bind |

### Performance monitoring

`scope benchmark` command produces a structured timing report:

```bash
scope benchmark --repo-root . --json
```

Output:
```json
{
  "schema_version": 1,
  "repo_root": ".",
  "files_scanned": 847,
  "timing": {
    "full_index_ms": 4821,
    "incremental_index_ms": 47,
    "scan_ms": 210,
    "parse_ms": 3100,
    "resolve_ms": 890,
    "db_write_ms": 621,
    "git_churn_ms": 1240,
    "query_latency_p50_ms": 8,
    "query_latency_p95_ms": 34,
    "query_latency_p99_ms": 89
  }
}
```

---

## 20. Risks and mitigations

| Risk | Severity | Problem | Mitigation |
|------|----------|---------|------------|
| Overpromising "full impact" | High | Users interpret static blast radius as guaranteed runtime correctness | Always position as "static analysis"; include certainty labels on every result; document known blind spots in README |
| Language complexity explosion | High | Every language has unique edge cases; scope becomes impossible to maintain | Adapter architecture strictly; one language family at a time; keep shared intermediate model narrow; reject contributions that widen it |
| False-positive call edges | High | Aggressive resolution creates misleading impact data and loses user trust | Conservative by default: prefer missing an edge over inventing one; `heuristic` certainty for anything uncertain; `exact` only for unambiguous syntax |
| Slow re-indexing on large repos | Medium | Fast queries meaningless if indexing takes minutes | blake3 hashing; rayon parallel parsing; benchmark early on large fixture; incremental invalidation; profiling gate in CI |
| Dynamic language blind spots | Medium | JS/TS runtime-dependent imports and dispatch | Mark as `dynamic` certainty; keep in separate "uncertain" group; document limitation; add framework adapters incrementally |
| `rename-plan --apply` data loss | High | Byte-offset rewriter corrupts files with concurrent edits or stale index | Dry-run default; sanity-check each substitution before applying (verify old_text matches); write atomically via temp file; abort on first mismatch |
| `arch.toml` false authority | Medium | Users treat violations as bugs when they are deliberate design choices | Frame as "rules you define"; violations are informational by default; `--strict` to make fatal; always show which rule was violated |
| Git log brittleness | Medium | `git log` unavailable, slow, or produces unexpected output | Make churn population optional (`--no-git`); all git-dependent features degrade gracefully to fan-in-only scores; detect git availability at startup |
| Cochange false positives | Low | Files always committed together due to CI tooling, not real coupling | Allow `.scope/cochange-ignore` glob patterns; always show commit count so user can evaluate; filter out commits touching > 50 files (likely tooling commits) |
| Simulate extract inaccuracy | Medium | In-memory graph mutation misses dynamic/heuristic edges | Clearly label simulation confidence; show which edges were excluded from simulation; recommend manual verification |
| `scope serve` security | Low | Local HTTP server accessible to other local processes | Bind to 127.0.0.1 only (never 0.0.0.0); no auth tokens needed for local dev; document this in README |
| `scope gate` brittleness | Medium | Teams disable gates when they interfere with velocity | Per-gate severity control; baseline comparison mode (only new regressions fail); easy override with `--skip-gate <name>` |
| Health score gaming | Low | Teams optimize the number rather than the underlying quality | Document formula completely; make it transparent; explicitly note "this is a lagging indicator, not a KPI" |
| Token count inaccuracy | Low | tiktoken-rs estimates differ from actual model tokenizer | Add 10% safety margin to budget; label as "approximate"; `scope pack` never claims exact token count |
| SQLite concurrency issues | Low | Multiple `scope` processes on same repo collide | WAL journal mode allows concurrent reads; writes are serialized naturally; detect and warn on concurrent write attempts |
| tree-sitter version drift | Medium | tree-sitter grammar updates break extraction | Pin grammar versions in Cargo.toml; run full golden tests on version bumps; keep test fixtures frozen |

---

## 21. Open questions

Resolve before or during the relevant milestone:

**Language and extraction:**
1. Should v1 support only Rust, or Rust + TS/JS at launch? (Recommendation: Rust first,
   TS/JS in M7; don't delay launch for multi-language.)
2. Should trait method dispatch be included in Rust v1, or deferred after plain function
   calls are solid? (Recommendation: defer, label trait calls `heuristic`.)
3. Should unresolved dynamic edges appear in default output or only behind `--verbose`?
   (Recommendation: separate "uncertain" section in default output, full list with --verbose.)
4. How to handle Rust macros? `println!()` calls are technically function calls.
   (Recommendation: extract macro call sites as `heuristic` certainty call edges.)

**Architecture:**
5. Should `scope` use a single SQLite DB per repo root, or allow workspace-level overrides?
   (Recommendation: single DB per repo root; `--db <path>` override for power users.)
6. Should the MCP wrapper be built immediately after JSON stabilization or only after CLI
   adoption proves the JSON contracts? (Recommendation: after M10, parallel with polish.)
7. Should `scope arch init` detect layers from import patterns as well as directory names?
   (Recommendation: directory names only in v1; import pattern detection is M2 feature.)

**Context and LLM integration:**
8. Should `scope context` accept free-text task descriptions or only `--target` flags in v1?
   (Recommendation: both; free-text in v1 with simple keyword extraction, documented as
   "best-effort".)
9. Should `scope rename-plan --apply` create a git commit automatically?
   (Recommendation: no; leave git operations to the user; agents can git commit themselves.)

**Temporal and simulation:**
10. Should `scope cochange` run automatically during every `scope index`, or only on explicit
    invocation? (Recommendation: run during index if git available, but only for new commits
    since last index — incremental, so usually fast.)
11. Should `scope simulate extract` be allowed to operate on partially-resolved graphs
    (i.e., where some imports have `heuristic` certainty)? (Recommendation: yes, but label
    the simulation confidence as `heuristic` when any input edges are heuristic.)

**Web UI and serve:**
12. Should `scope serve` embed the entire web UI as a compiled-in static asset, or require
    a separate install step? (Recommendation: compiled-in as `include_str!` — zero config
    for users, ~500KB binary size increase is acceptable.)
13. Should `scope query` REPL support user-defined functions/macros, or stay intentionally
    minimal? (Recommendation: minimal in v1 — named queries only, no user functions.)

**Gates and CI:**
14. Should `scope gate --compare <branch>` require a pre-existing snapshot of the base
    branch, or build it on-demand? (Recommendation: require pre-existing; on-demand is too
    slow for CI. Document the CI setup steps.)

---

## 22. Timeline

### Summary

| Milestone | Description | Estimated Duration |
|-----------|-------------|-------------------|
| Phase 0 | Node.js POC | ~1 day |
| M0 | Foundation: workspace, fixtures, CLI stubs | ~1 day |
| M1 | File dependency graph | ~1–2 days |
| M2 | Symbol inventory | ~1 day |
| M3 | Direct call graph | ~1–2 days |
| M4 | Impact engine | ~1–2 days |
| M5 | Path and context queries | ~1 day |
| M6 | Incremental indexing | ~1 day |
| M7 | TypeScript/JavaScript adapter | ~1–2 days |
| M8 | Architectural analysis | ~1–2 days |
| M9 | Surface, rename plan, test map | ~1–2 days |
| M10 | Snapshots and agent integration | ~1 day |
| M11 | Extra utilities and polish | ~1 day |
| M12 | Temporal coupling and simulation | ~2 days |
| M13 | Entry points, audit, decomposition | ~2 days |
| M14 | Report, gates, serve, REPL | ~2 days |
| **Total** | | **~22–28 days** |

### Phasing philosophy

- **Milestones 0–6** (core product): After M6, `scope index`, `scope deps`, `scope symbols`,
  `scope callers`, `scope impact`, `scope why`, and `scope context` all work for Rust.
  This is a shippable v0.1 for early adopters.

- **Milestones 7–10** (the exceptional product): After M10, the tool works for
  TypeScript/JavaScript, enforces architecture rules, analyzes risk and stability, and
  integrates with Claude Code via MCP. This is v0.5.

- **Milestones 11–14** (the platform): After M14, all 20 features from Rounds 1 and 2 are
  implemented. This is v1.0.

Quality of graph and schema stability matter more than hitting calendar dates. Ship M6
early and get feedback before building M7–M14.

---

## 23. Definition of done for v1.0

`scope` v1.0 is done when all of the following are true:

**Core functionality:**
1. A user can `scope index` any Rust or TypeScript/JavaScript repository locally.
2. `scope deps`, `scope symbols`, `scope calls`, `scope callers` all return correct,
   explainable results with certainty labels.
3. `scope impact --change-type` correctly identifies blast radius for all 6 change types.
4. `scope why` correctly traces dependency paths between any two connected nodes.
5. `scope context` produces a correctly ranked minimum read set for a task.
6. `scope pack` respects the token budget and produces well-structured LLM context.

**Quality:**
7. All commands support `--json` with stable, versioned schemas.
8. Output includes `reason`, `certainty`, and `distance` where relevant.
9. Incremental indexing works using blake3 content hashing.
10. Parse errors are surfaced, never silently dropped.

**Architectural analysis:**
11. `scope arch check` enforces user-defined layer rules and exits non-zero in CI.
12. `scope risk` correctly surfaces churn-weighted hotspots.
13. `scope stability` correctly computes Martin instability index.
14. `scope surface diff` correctly identifies breaking vs non-breaking changes.
15. `scope rename-plan --apply` correctly renames symbols across a codebase.

**Advanced features:**
16. `scope cochange` correctly identifies unexpected temporal coupling pairs.
17. `scope simulate extract` correctly previews graph deltas for proposed extractions.
18. `scope entry unreachable` correctly identifies provably dead files.
19. `scope audit --capability` correctly traces unexpected capability reach paths.
20. `scope split` produces valid decomposition suggestions.
21. `scope report` produces a correct composite health score.
22. `scope gate` enforces threshold rules and exits non-zero on violations.
23. `scope serve` starts cleanly and all API endpoints return correct JSON.
24. `scope query` REPL evaluates all language primitives correctly.

**Packaging and docs:**
25. All fixture repos have golden JSON tests that pass in CI.
26. Documentation clearly states supported patterns and known limitations.
27. `CLAUDE.md` integration snippet tested with a real Claude Code agent session.
28. `cargo install scope-cli` works on macOS, Linux, and Windows.
29. README includes quick-start guide from install to first useful query in < 5 minutes.

---

## 24. First execution checklist

### Foundation
- [ ] Create Cargo workspace with `scope-core`, `scope-cli`, `scope-mcp` crate stubs
- [ ] Add all workspace dependencies to root `Cargo.toml`
- [ ] Set up `config.rs`: repo root detection, `.scope/` directory management
- [ ] Set up `store.rs`: DB open, WAL mode, foreign keys, migration runner
- [ ] Write `001_initial.sql` migration with complete schema
- [ ] Set up `output.rs`: `JsonEnvelope<T>`, human formatter trait
- [ ] Set up `tracing_subscriber` initialization
- [ ] Define all `ScopeError` variants
- [ ] Create all CLI command stubs with `clap` derive
- [ ] Create all fixture repo directories with source files
- [ ] Verify `scope --help` and all subcommand `--help` work

### Indexing (M1–M3)
- [ ] Implement `scanner.rs`: file walker using `ignore` crate
- [ ] Implement `adapters/mod.rs`: `Adapter` trait + all normalized types
- [ ] Implement `adapters/rust.rs`: import extraction
- [ ] Implement `adapters/rust.rs`: symbol extraction
- [ ] Implement `adapters/rust.rs`: call site extraction
- [ ] Implement `resolver.rs`: relative import path resolution
- [ ] Implement `store.rs`: all CRUD operations for all tables
- [ ] Implement `scope index` command (full)
- [ ] Add blake3 hashing + incremental detection

### Core queries (M1–M5)
- [ ] `scope deps` / `scope deps --reverse` / `scope deps --transitive`
- [ ] `scope symbols` with `--public-only` and `--kind` filters
- [ ] `scope calls` / `scope callers` with `--transitive`
- [ ] `scope impact` for all 6 change types
- [ ] `scope explain` with full evidence trail
- [ ] `scope why` with Dijkstra + Yen's k-shortest
- [ ] `scope context` with BFS + scoring + classification
- [ ] `scope pack` with tiktoken-rs budget enforcement
- [ ] `scope doctor`
- [ ] `scope benchmark`

### TypeScript/JavaScript (M7)
- [ ] `adapters/javascript.rs`: ES module imports/exports
- [ ] `adapters/javascript.rs`: CommonJS require
- [ ] `adapters/typescript.rs`: TypeScript-specific types
- [ ] Barrel file detection and re-export tracing
- [ ] TypeScript path resolution from `tsconfig.json`
- [ ] Dynamic import/require detection → `dynamic` certainty

### Architectural analysis (M8–M9)
- [ ] `arch.rs`: `arch.toml` parser + layer pattern matcher
- [ ] `arch.rs`: violation detector + `arch check` + `arch init`
- [ ] `stability.rs`: Martin instability metric per file
- [ ] `risk.rs`: git log churn population + risk score formula
- [ ] `surface.rs`: public surface extraction + diff
- [ ] `rename_plan.rs`: topological plan + byte-offset safe `--apply`
- [ ] `test_map.rs`: test file detection + BFS coverage map

### Snapshots and agent integration (M10)
- [ ] `snapshot.rs`: save/load/delete/list with zstd compression
- [ ] `snapshot.rs`: diff between two snapshots
- [ ] `scope-mcp`: MCP stdio server wrapping all core commands
- [ ] Write and test `CLAUDE.md` integration guide

### Round 2 features (M12–M14)
- [ ] `cochange.rs`: co-occurrence matrix + rate computation + unexpected classification
- [ ] `simulate.rs`: in-memory graph clone + mutation + stability projection
- [ ] `entry.rs`: entry point detection + cone BFS + unreachable computation
- [ ] `audit.rs`: capability tags + reverse-BFS reach + unexpected path detection
- [ ] `split.rs`: caller profile extraction + Jaccard clustering + module naming
- [ ] `mirror.rs`: graph signature vectors + weighted Jaccard similarity
- [ ] `report.rs`: health report aggregation + health score formula + Markdown renderer
- [ ] `gate.rs`: gates.toml parser + metric evaluation + delta gates + exit codes
- [ ] `serve.rs`: Axum HTTP server + all API endpoints + CORS
- [ ] Web UI: D3 force graph + all overlay modes + search (single HTML file)
- [ ] `query_lang.rs`: lexer + recursive descent parser + evaluator
- [ ] REPL: rustyline integration + tab completion + history

### Quality
- [ ] Golden JSON tests for all commands on all fixtures
- [ ] Arch violation fixture tests (check exit code 0/1)
- [ ] Rename plan fixture tests (apply mode round-trip)
- [ ] Cochange fixture tests (matrix accuracy)
- [ ] Gate fixture tests (pass/fail scenarios)
- [ ] Performance benchmarks passing all targets in §19
- [ ] Limitations documentation: what scope cannot detect
- [ ] `CLAUDE.md` tested with Claude Code in a real session

---

## 25. Configuration reference

### `.scope/arch.toml` complete schema

```toml
# ─── ARCHITECTURAL LAYERS ─────────────────────────────────────────────────
# Each [[layer]] defines a named group of files matched by glob pattern.
# Files not matching any layer are "unlayered" and not checked against rules.

[[layer]]
name = "routes"                     # Layer name (used in rules)
pattern = "src/routes/**"           # Glob pattern relative to repo root
description = "HTTP route handlers" # Optional documentation

# ─── RULES ────────────────────────────────────────────────────────────────
# Each [[rule]] forbids import edges between layers.

[[rule]]
from = "utils"                              # Source layer name
may_not_import = ["routes", "services"]     # Forbidden target layer names
message = "utils must be pure"              # Optional: shown in violation output

# ─── CAPABILITIES ─────────────────────────────────────────────────────────
# Each [[capability]] defines a set of sensitive operations.

[[capability]]
name = "network"                    # Capability identifier (used with scope audit)
pattern = "src/http/**"             # Files that ARE this capability
symbols = ["fetch", "axios.get"]    # Function names that ARE this capability
# Files/entries that are EXPECTED to reach this capability (patterns allowed)
expected_callers = ["src/server.js", "src/workers/**"]

# ─── ENTRY POINTS ─────────────────────────────────────────────────────────
# Override auto-detection of entry points for scope entry.

[[entry_point]]
pattern = "src/server.*"

[[entry_point]]
pattern = "src/cli.*"

# ─── TEST FILE PATTERNS ───────────────────────────────────────────────────
# Customize how scope detects test files for scope test-map.

[tests]
patterns = [
  "**/*.test.*",
  "**/*.spec.*",
  "tests/**",
  "**/test_*.rs",
  "**/__tests__/**",
]
exclude_patterns = [
  "**/__mocks__/**",  # Mock files are not real tests
]

# ─── CO-CHANGE IGNORE ─────────────────────────────────────────────────────
# Suppress co-change pairs involving these patterns (e.g., generated files).

[cochange]
ignore_patterns = [
  "*.generated.*",
  "src/migrations/**",   # All migrations always change together — expected
]
min_shared_commits = 5   # Require at least 5 shared commits before flagging
```

### `.scope/gates.toml` complete schema

```toml
# Each [[gate]] defines one metric threshold for CI enforcement.

[[gate]]
metric = "layer_violations"     # Required: metric name (see full list in §16.18)
max = 0                         # Maximum allowed value (use for count metrics)
# min = 80                      # Minimum allowed value (use for score metrics)
# min_delta = -5                # Minimum allowed change vs --compare ref
# max_delta = 10                # Maximum allowed change vs --compare ref
severity = "error"              # "error" (fails CI) or "warning" (logs only)
message = "Fix before merging"  # Optional: shown in gate output
# skip = false                  # Set to true to temporarily disable this gate
```

---

## 26. Integration with Claude Code

Add to any project's `CLAUDE.md` to enable `scope` integration:

```markdown
## scope — Dependency Graph Tool

scope is indexed at `.scope/index.db`. Run `scope index` if the index seems stale
(older than your most recent file changes).

### Before editing any file:
1. `scope context "<description of what you're doing>"` — get the minimum read set.
   Only read the files it lists as `must_read` before making changes.
2. `scope pack <target> --budget 4000` — get a pre-formatted context payload.
   Paste this directly into your working context instead of reading files manually.
3. `scope impact <path> --change-type <type>` — know the blast radius before editing.
4. `scope symbols <path>` — see the public surface you must preserve.

### After editing:
5. `scope why <changed-file> <failing-test>` — trace unexpected breakage.
6. `scope surface diff HEAD~1 HEAD` — verify no unintended public API changes.
7. `scope gate` — confirm no architectural regressions.
8. `scope test-map covers <changed-file>` — know exactly which tests to run.

### For exploration and refactoring:
9. `scope serve --open` — open the visual graph explorer in the browser.
10. `scope query` — interactively explore the dependency graph.
11. `scope simulate extract <symbols> --into <new-file>` — validate a refactoring
    plan before making any changes.
12. `scope split <file>` — get decomposition suggestions for oversized files.
13. `scope cochange --file <path>` — discover hidden coupling partners.

### Change type reference:
- `body` — implementation changes only, signature unchanged
- `signature` — parameter types, return type, or contract changed
- `rename` — symbol or file name changes
- `delete` — symbol or file will be removed
- `visibility` — pub/private accessibility changes
- `side-effect` — module-level initialization behavior changes
```

---

## 27. One-sentence summary

`scope` is a local Rust static-analysis engine that indexes repo files, symbols, and
resolvable calls into SQLite so developers and coding agents can ask dependency and
blast-radius questions instantly — discover hidden temporal coupling, simulate
refactorings before touching code, enforce architectural rules in CI, hunt dead code,
audit capability reach for security, and explore the full graph interactively — all
with structured, explainable, confidence-labeled results and zero LLM or external API
dependencies.

