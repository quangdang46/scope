# scope

> Local static-analysis workspace for dependency, impact, and architecture queries.
>
> Today, `scope` is a working local static-analysis workspace with a SQLite-backed index, a broad Rust CLI, a local HTTP API/UI, and a stdio MCP wrapper. The strongest semantic indexing support today is heuristic Rust and TS/JS extraction; parser-backed resolution, tighter traversal precision, and richer module resolution are still in progress.

---

## Current Status

The repository currently implements:

- a Rust workspace with `scope-core`, `scope-cli`, and `scope-mcp`
- bootstrap logic for discovering the repo root and creating `.scope/index.db`
- SQLite initialization, schema version tracking, and persisted file/import/symbol/symbol-edge/churn/snapshot records
- ignore-aware repository scanning across Rust, TS/JS, Python, Ruby, and Go file types
- heuristic semantic extraction for Rust and TS/JS imports, exports, symbols, visibility, and direct call sites
- machine-readable CLI commands for dependency, symbol, impact, architecture, report/gate, diff, and simulation workflows
- a local HTTP `/api/*` server with a bundled embedded UI
- a stdio MCP tool server exposing the same core store/query surface to external clients
- fixture-based golden tests plus CLI integration coverage for the current contract

The repository does **not** yet implement:

- tree-sitter or other AST-backed parsing
- parser-backed precision improvements for traversal and impact analysis
- semantic extraction adapters for Python, Ruby, or Go (those file types are scanned but not yet indexed deeply)
- tsconfig path mapping and other richer module-resolution cases
- the planned query-REPL ergonomics such as history, completion, and saved-query workflows

## The Goal

Before editing a file, an agent should be able to ask:

- What does this file depend on?
- What depends on this file?
- Which symbols are defined here?
- What calls this symbol?
- What is the likely blast radius of a change?

`scope` is being built to answer those questions from a local persistent index rather than from repeated manual file reads.

## Architecture Direction

The intended architecture is:

```text
1. Bootstrap runtime state
   └── discover repo root
   └── create .scope/
   └── open SQLite index

2. Index source files
   └── walk source files with ignore-aware scanning
   └── extract imports / exports / symbols / call sites from supported Rust and TS/JS files
   └── persist file, symbol, call-edge, and related analysis data

3. Query the graph
   └── deps / symbols / calls / callers / impact / explain / why / context work today
   └── report / gate / serve / MCP reuse the same core store queries
   └── richer resolution and precision improvements remain planned
```

The current codebase has working stage-2 and stage-3 slices across the CLI, local HTTP server, and stdio MCP wrapper, but parser fidelity and several resolution/precision features are still incomplete.

## Current Tech Stack

### Implemented now

| Crate | Purpose |
|---|---|
| `clap` | CLI parsing |
| `rusqlite` | Local SQLite storage |
| `serde` / `serde_json` | Machine-readable output |
| `tracing` / `tracing-subscriber` | Logging |
| `thiserror` | Error handling |

### Planned / not implemented yet

| Crate / capability | Intended purpose |
|---|---|
| `tree-sitter` + grammars | Replace heuristic parsing with stronger syntax-backed extraction |
| graph traversal library | Power transitive dependency, impact, and explain queries |
| multi-language adapters | Extend indexing beyond the current Rust-first implementation |

## Workspace Layout

```text
scope/
├── crates/
│   ├── scope-cli/    # CLI entrypoint
│   ├── scope-core/   # shared models, bootstrap, storage, query/report/serve logic
│   └── scope-mcp/    # stdio MCP/JSON-RPC wrapper over scope-core
└── .scope/
    └── index.db      # local SQLite index database
```

## Command Surface

The current CLI exposes these command families:

```bash
scope index [PATH]
scope deps <file> [--reverse] [--transitive] [--depth N]
scope symbols <file> [--public-only] [--kind ...]
scope calls <symbol> [--transitive]
scope callers <symbol> [--transitive]
scope impact <target> --change-type <body|signature|rename|delete|visibility|side-effect>
scope explain <target> [--to ...] [--depth N]
scope why <from> <to> [--depth N]
scope context --target <symbol-or-file>... --change-type <...> [--budget N]
scope pack <target> --change-type <...> --budget <N>
scope arch check | scope audit --capability <name> | scope surface [diff]
scope stability | risk | cochange | test-map | rename-plan | snapshot | diff-snapshot
scope simulate extract ... | report | gate | unused | cycles | diff | tree | split | mirror | entry
scope serve [--port 7777] [--no-ui]
scope query [--expr ...]
scope doctor [--fix] | benchmark [--fixture ...] [--iterations N] [--write-report]
```

Right now:

- `scope index` scans supported files, indexes Rust and TS/JS extracts, and refreshes dependency/call edges in `.scope/index.db`
- `scope deps`, `scope symbols`, `scope calls`, `scope callers`, `scope impact`, `scope explain`, `scope why`, `scope context`, and `scope pack` return structured envelopes from the SQLite-backed graph
- architecture, audit, surface, risk/stability/cochange, test-map, snapshot/diff, simulation, report/gate, unused/cycles, entry, tree/split/mirror, doctor, and benchmark commands are all wired through the CLI today
- `scope serve` exposes the same core analyses through `/api/*` endpoints with a bundled embedded UI
- `scope query` supports both single-expression execution and a basic interactive REPL over the query language
- `scope-mcp` exposes core operations as stdio MCP tools backed by the same store/query layer
- `scope benchmark --write-report` writes a linehash-style Markdown summary to `bench-results/benchmark.md` plus a timestamped `bench-results/bench-YYYY-MM-DD-HH-MM-SS.md` snapshot

## Agent Usage Patterns

`scope` is intended to answer repository-structure questions before an agent starts editing code.

### Recommended workflow for coding agents

1. Run `scope index .` once for the repository snapshot you are working against.
2. Ask the narrowest query that answers the immediate planning question.
3. Prefer `--compact` for machine consumption when the result will be fed back into an agent loop.
4. Treat the result as static evidence, not as proof that a change is safe.
5. Use tests, builds, and human review to validate changes after editing.

### Typical pre-edit questions

```bash
scope deps src/lib.rs
scope --compact symbols src/parser.rs --public-only
scope --compact callers parser::parse
scope --compact context --target parser::parse --change-type body --budget 400
scope pack parser::parse --change-type body --budget 400
```

Use them like this:

- `deps` / `--reverse` to understand direct file-level coupling
- `symbols` to see what a file defines before editing it
- `calls` / `callers` to understand direct symbol-level interactions
- `impact` / `explain` / `why` / `context` when you need structured change-planning evidence
- `pack` when you want a lean plain-text handoff for an agent prompt

### Output expectations

Machine-readable commands return a stable JSON envelope:

- `schema_version` — contract version for downstream tooling
- `command` — command name that produced the result
- `status` — `ok`, `stub`, or `error`
- `data` — command-specific payload
- `warnings` — non-fatal notes

Default output is pretty-printed JSON for readability.

`--compact` keeps the same top-level JSON contract while reducing token cost:

- emits minified JSON instead of pretty JSON
- prunes null and empty nested fields from the payload where possible
- keeps essential graph facts such as paths, reasons, certainty, and command metadata

Compact mode is intended for agent loops and transport efficiency, not for humans reading terminal output.

### Current limitations for agents

- semantic extraction is strongest today for Rust and TS/JS; Python, Ruby, and Go are currently scan-only
- transitive `deps`, `calls`, and `callers` are available today, but they remain static approximations built on heuristic extraction and conservative resolution
- parsing and resolution are still heuristic and may miss language-specific edge cases
- TypeScript module resolution is conservative and does not yet cover `tsconfig` path mapping
- the query REPL exists, but it does not yet implement the planned history/completion/save-load ergonomics
- results are static approximations and may omit dynamic behavior
- `scope-mcp` is functional today, but the CLI remains the most mature integration surface

## Example Current Output

### `scope index .`

```json
{
  "schema_version": 1,
  "command": "index",
  "status": "ok",
  "data": {
    "repo_root": ".",
    "no_git": false,
    "watch": false,
    "database": {
      "path": ".scope/index.db",
      "schema_version": 1
    }
  },
  "warnings": []
}
```

### `scope deps src/lib.rs`

```json
{
  "schema_version": 1,
  "command": "deps",
  "status": "ok",
  "data": {
    "target": "src/lib.rs",
    "reverse": false,
    "transitive": false,
    "depth": null,
    "dependencies": [
      {
        "path": "src/parser.rs",
        "kind": "import",
        "certainty": "exact"
      },
      {
        "path": "src/resolver.rs",
        "kind": "import",
        "certainty": "exact"
      }
    ]
  },
  "warnings": []
}
```

## Data Model Direction

`scope-core` already defines shared records for the planned engine, including:

- files and parse status
- imports and exports
- symbols and visibility
- call sites
- dependency traversal records
- certainty levels such as `exact`, `resolved`, `heuristic`, and `dynamic`

This model is intended to support machine-first static analysis without implying runtime guarantees.

## Installation

This project is still under active development and is not yet ready to advertise as a finished package.

For local development:

```bash
cargo run -p scope-cli -- --help
cargo run -p scope-cli -- index .
cargo run -p scope-cli -- --compact deps src/lib.rs
cargo run -p scope-cli -- report
cargo run -p scope-cli -- serve --port 7777
cargo run -p scope-mcp
```

## Near-Term Roadmap

- replace heuristic Rust and TS/JS extraction with parser-backed logic
- tighten transitive traversal precision for `deps`, `calls`, and `callers`
- persist richer diagnostics/export data and tighten certainty reporting
- add semantic adapters beyond Rust and TS/JS, including better TypeScript module resolution
- improve the query REPL with history, completion, and saved-query workflows
- deepen MCP polish/coverage while keeping JSON contracts stable for agent consumption

## Scope Boundaries

`scope` is intended to provide **static** dependency and impact insight.

It should not be described as:

- a runtime behavior oracle
- a guarantee that a change is safe
- a substitute for tests, builds, or human review

Results should be interpreted as structured static evidence with explicit certainty levels.

## Intended Certainty Model

The planned analysis model uses four certainty levels:

- `exact` — directly supported by unambiguous syntax or deterministic resolution
- `resolved` — strongly supported by repository context, but requires some inference
- `heuristic` — plausible and useful, but not guaranteed
- `dynamic` — known blind spot or unresolved dynamic behavior

The intended rule is conservative resolution:

- prefer missing a low-confidence edge over inventing a false one
- reserve `exact` for unambiguous evidence
- surface uncertainty in results instead of hiding it

This is especially important for impact analysis, where false positives can be more damaging than incomplete-but-honest results.

## Intended Blind Spots and Limitations

Even after the engine is implemented, some classes of behavior should be treated as uncertain or only partially modeled:

- dynamic imports / computed module paths
- reflection and metaprogramming
- macro expansion and generated code
- dynamic dispatch patterns that cannot be resolved statically
- framework-specific conventions that require adapters not yet implemented
- unsupported languages or syntax the parser cannot fully interpret

The project direction in `PLAN.md` is to label these cases explicitly rather than pretend they are fully understood.

## Intended Failure-Handling Principles

The project direction also includes a few important trust rules:

- do not silently drop parse problems; surface partial results and diagnostics
- do not crash on unsupported syntax when partial results are possible
- do not overclaim “full impact” when the result is only a static approximation
- keep JSON output machine-readable and stable as the contract evolves
