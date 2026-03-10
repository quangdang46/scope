# scope

> Local static-analysis workspace for dependency and impact queries.
>
> Today, `scope` is an early Rust-first prototype: it can scan a repository, build a local SQLite index for supported Rust files, and answer machine-readable `deps`, `symbols`, `calls`, and `callers` queries. Transitive traversal, impact analysis, richer parsing, and a real MCP server are still in progress.

---

## Current Status

The repository currently implements:

- a Rust workspace with `scope-core`, `scope-cli`, and `scope-mcp`
- bootstrap logic for discovering the repo root and creating `.scope/index.db`
- SQLite initialization, schema version tracking, and persisted file/symbol/call-edge records
- ignore-aware repository scanning for supported source file types
- heuristic Rust extraction for modules, imports, symbols, visibility, and direct call sites
- machine-readable CLI queries for direct file dependencies, symbol inventory, and direct callers/callees
- fixture-based golden tests for current query behavior

The repository does **not** yet implement:

- tree-sitter or other AST-backed parsing
- transitive call traversal
- impact or explain traversal
- non-Rust indexing adapters in the CLI path
- persisted parse diagnostics / export records for queries
- a working MCP server

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
   └── extract imports / symbols / call sites from supported Rust files
   └── persist file, symbol, and direct call-edge data

3. Query the graph
   └── deps / symbols / direct calls / direct callers work today
   └── impact / explain / transitive traversal remain planned
   └── return structured JSON for agents and tooling
```

The current codebase has working stage-2 and early stage-3 slices for Rust, but larger graph traversal and explanation features are still scaffolded.

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
│   ├── scope-core/   # shared models, bootstrap, storage, JSON contracts
│   └── scope-mcp/    # future MCP server (currently stubbed)
└── .scope/
    └── index.db      # local SQLite database
```

## Command Surface

The current CLI exposes these commands:

```bash
scope index [PATH]
scope deps <file>
scope symbols <file>
scope calls <symbol>
scope callers <symbol>
scope impact <target> --change-type <body|signature|rename|delete|visibility|side-effect>
```

Right now:

- `scope index` scans the repo, indexes supported Rust files, and refreshes direct call edges in `.scope/index.db`
- `scope deps` returns direct forward and reverse file dependencies from the SQLite index
- `scope symbols` returns indexed symbols, with `--public-only` and `--kind` filtering
- `scope calls` and `scope callers` return direct, conservative call edges for supported Rust cases
- `scope impact`, `scope explain`, `scope doctor`, and `scope benchmark` still return scaffolded stub JSON

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

- `scope` is currently Rust-first in the main CLI path
- several traversal-oriented commands still return stub JSON while implementation is in progress
- results are static approximations and may omit dynamic behavior
- `scope-mcp` is still a stub, so the supported integration surface today is the CLI

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
```

## Near-Term Roadmap

- strengthen Rust extraction with parser-backed logic instead of line heuristics
- implement transitive traversal for calls / callers / impact / explain
- persist additional analysis data such as exports and parse diagnostics
- add non-Rust adapters and wire them into the indexing pipeline
- replace stub MCP output with a real MCP integration
- keep JSON contracts stable for agent consumption
- add compact JSON response shaping for token-sensitive agent workflows

## Scope Boundaries

`scope` is intended to provide **static** dependency and impact insight.

It should not be described as:

- a runtime behavior oracle
- a guarantee that a change is safe
- a substitute for tests, builds, or human review

When the engine lands, results should be interpreted as structured static evidence with explicit certainty levels.

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
