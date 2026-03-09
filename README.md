# scope

> Local static-analysis workspace for dependency and impact queries.
>
> Today, `scope` is an early Rust scaffold: it can bootstrap a local SQLite index and expose a machine-readable CLI surface, but the parser, graph builder, and real query engine are still in progress.

---

## Current Status

The repository currently implements:

- a Rust workspace with `scope-core`, `scope-cli`, and `scope-mcp`
- bootstrap logic for discovering the repo root and creating `.scope/index.db`
- SQLite initialization and schema version tracking
- a JSON-first CLI surface for planned query commands
- stub responses for query commands while the analysis engine is being built

The repository does **not** yet implement:

- source walking / `.gitignore`-aware indexing
- tree-sitter parsing
- dependency graph construction
- symbol or call resolution
- impact traversal
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

2. Index source files   (planned)
   └── walk source files
   └── parse files into imports / symbols / call sites
   └── persist file and symbol graph data

3. Query the graph      (planned)
   └── deps / symbols / calls / callers / impact
   └── return structured JSON for agents and tooling
```

The current codebase is mostly stage 1 plus command scaffolding for stages 2 and 3.

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
| `tree-sitter` + grammars | Parse imports / exports / call sites |
| graph traversal library | Dependency and impact traversal |
| ignore-aware walking | Respect project boundaries during indexing |

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

- `scope index` bootstraps `.scope/index.db`
- query commands return structured stub JSON so downstream tooling can integrate before the engine is complete

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
  "status": "stub",
  "data": {
    "target": "src/lib.rs",
    "reverse": false,
    "transitive": false,
    "depth": null,
    "dependencies": []
  },
  "warnings": [
    "Command is scaffolded but not implemented yet"
  ]
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
```

## Near-Term Roadmap

- implement file walking and indexing
- persist file nodes and dependency edges to SQLite
- implement symbol inventory queries
- implement calls / callers queries
- implement impact traversal
- replace stub MCP output with a real MCP integration
- keep JSON contracts stable for agent consumption

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
