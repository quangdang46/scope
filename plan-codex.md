# scope - Detailed Plan

## 1. Overview

### Product statement

`scope` is a local static-analysis engine and CLI for coding agents and developers. It indexes a repository once, persists a dependency graph locally, and answers file, symbol, and change-impact questions in milliseconds.

### Core promise

Before editing a file or function, a user or agent can ask:

- What does this file import?
- Who imports this file?
- What symbols does this file expose?
- Which symbols are public vs internal?
- Who calls this function?
- If I rename, delete, or change this, what else is likely affected?

### Positioning

`scope` does **not** try to predict exact runtime behavior. It provides a **static blast-radius analysis** with structured results, clear limitations, and confidence levels.

### Why this matters

Without an index, coding agents answer dependency questions by opening file after file, following import chains manually, and spending context budget before any real work begins. `scope` replaces those repeated file reads with near-zero-token structured graph queries.

---

## 2. Goals

### Primary goals

1. Build a local index of repository dependencies and symbols.
2. Answer dependency and impact queries instantly from SQLite.
3. Produce structured JSON designed for LLM/tool consumption.
4. Work fully offline with no LLM or API calls.
5. Be fast enough to run as a normal part of an edit loop.

### Success criteria

`scope` is successful when:

- A developer can answer common dependency questions without opening multiple files.
- A coding agent can ask `scope` for impact data instead of reading 5–20 files.
- Query latency feels instant in normal use.
- Results are trustworthy because they include reason trails and confidence labels.

### Non-goals for v1

- Exact runtime breakage prediction.
- Deep type-system semantic analysis across every language feature.
- Perfect resolution of reflection, dynamic imports, macros, or generated code.
- External package internals.
- Cross-repo analysis.
- IDE-grade refactoring or code edits.

---

## 3. Users and jobs-to-be-done

### Primary users

- Coding agents such as Claude Code.
- Developers working in medium and large monorepos.
- Teams that need quick impact analysis before edits.

### Jobs to be done

1. **Pre-edit understanding**
   - “What is this file connected to?”
   - “Is this symbol public?”

2. **Change planning**
   - “If I rename this function, what needs to change?”
   - “If I delete this file, what depends on it?”

3. **Refactor safety**
   - “What is the static blast radius of this signature change?”
   - “Which callers are most likely affected?”

4. **Agent efficiency**
   - Replace many file reads with one graph query.

---

## 4. Product boundaries and assumptions

### Key assumption

The highest-value output is not “the exact truth of runtime behavior”; it is “the best static graph-based answer with confidence and evidence.”

### Product principle

Every query should prefer:

- explicit graph evidence,
- transparent limitations,
- machine-readable output,
- stable CLI behavior.

### Accuracy model

`scope` should surface certainty for every edge and impacted node.

Supported certainty levels:

- `exact` - directly known from syntax or deterministic resolution.
- `resolved` - strongly resolved within repo context.
- `heuristic` - inferred but not fully guaranteed.
- `dynamic` - known blind spot or unresolved dynamic behavior.

---

## 5. Supported concepts

### Entities

- **File**: a source file inside the repository.
- **Symbol**: a function, method, class, type, module, or exported constant.
- **Import edge**: file A depends on file B.
- **Call edge**: symbol A calls symbol B.
- **Export**: symbol is part of the file/module public surface.
- **Visibility**: local/module/package/public/unknown.
- **Impact set**: nodes likely affected by a change.

### Query targets

v1 query targets should support:

- file path
- qualified symbol name

Examples:

```bash
scope deps src/parser/mod.rs
scope calls crate::resolver::resolve_symbol
scope impact crate::resolver::resolve_symbol --change-type signature
```

---

## 6. MVP definition

### Must-have in MVP

1. Repository walk respecting `.gitignore`.
2. SQLite-backed persistent index.
3. File-to-file dependency graph.
4. Reverse dependency queries.
5. Top-level symbol extraction.
6. Visibility/public surface classification.
7. Direct in-repo call graph for resolvable calls.
8. Impact analysis based on file or symbol and change type.
9. JSON output for agents and readable output for humans.
10. Incremental re-indexing for changed files.

### Should-have for MVP

- transitive dependency traversal
- reason trails in impact output
- content hashing to skip unchanged files
- fixture-based tests
- benchmark command

### Nice-to-have after MVP

- watch mode
- MCP server wrapper
- graph export to DOT/Graphviz
- IDE integration
- framework-specific analyzers
- language-server style daemon

---

## 7. Language strategy

### Recommendation

Start with **one language family only**.

Preferred rollout strategy:

1. Rust first if the initial product and users are Rust-heavy.
2. TypeScript/JavaScript second.
3. Additional languages only after the core model stabilizes.

### Why

A broad “any language” claim will create quality problems early. The core engine should be language-agnostic, but language-specific extraction must be adapter-based and introduced gradually.

### Adapter principle

Each language adapter normalizes syntax into a shared intermediate model.

Normalized fields must include:

- imports
- exports
- symbol definitions
- symbol kind
- visibility
- call sites
- source spans
- certainty

---

## 8. User experience and CLI contract

### CLI principles

- predictable commands
- stable JSON
- concise human-readable output
- explicit flags for transitive depth and change type

### Proposed commands

```bash
scope index
scope index --watch
scope deps <file>
scope deps <file> --reverse
scope deps <file> --transitive
scope symbols <file>
scope calls <symbol>
scope callers <symbol>
scope impact <target> --change-type <type>
scope explain <target>
scope doctor
scope benchmark
```

### Important flags

```bash
--json
--depth <n>
--transitive
--change-type <body|signature|rename|delete|visibility|side-effect>
--language <rust|ts|js>
--repo-root <path>
--db <path>
```

### Human-readable output examples

```bash
$ scope deps src/resolver.rs --reverse
Imported by:
- src/cli.rs
- src/index/mod.rs
- src/impact.rs
```

```bash
$ scope impact crate::resolver::resolve_symbol --change-type signature
Affected (high confidence):
- crate::impact::compute_impact   reason: calls target directly
- crate::cli::run_query           reason: calls impacted function

Affected (uncertain):
- crate::mcp::handle_request      reason: dynamic dispatch path includes target module
```

---

## 9. JSON contract

### Design principles

- compact enough for LLM/tool consumption
- stable across versions
- includes explanation fields
- includes certainty and traversal distance

### Example: `deps --json`

```json
{
  "target": "src/resolver.rs",
  "target_kind": "file",
  "dependencies": [
    {
      "kind": "file",
      "path": "src/parser.rs",
      "edge_kind": "import",
      "certainty": "exact"
    }
  ]
}
```

### Example: `symbols --json`

```json
{
  "file": "src/resolver.rs",
  "symbols": [
    {
      "qualname": "crate::resolver::resolve_symbol",
      "name": "resolve_symbol",
      "kind": "function",
      "visibility": "public",
      "exported": true,
      "span": {
        "start": 120,
        "end": 240
      }
    }
  ]
}
```

### Example: `impact --json`

```json
{
  "target": "crate::resolver::resolve_symbol",
  "target_kind": "function",
  "change_type": "signature",
  "affected": [
    {
      "kind": "function",
      "name": "crate::impact::compute_impact",
      "file": "src/impact.rs",
      "reason": "calls crate::resolver::resolve_symbol",
      "distance": 1,
      "certainty": "resolved"
    },
    {
      "kind": "function",
      "name": "crate::cli::run_query",
      "file": "src/cli.rs",
      "reason": "calls impacted function transitively",
      "distance": 2,
      "certainty": "resolved"
    }
  ],
  "summary": {
    "high_confidence": 2,
    "uncertain": 0
  }
}
```

### Versioning

All JSON responses should include a schema version in v1.

Example:

```json
{
  "schema_version": 1
}
```

---

## 10. Change-impact model

### Why change type matters

Impact is only meaningful if the tool knows **what kind of change** is being made. A function body change has a different blast radius from a rename or signature change.

### Supported change types

#### `body`

Meaning: implementation changes but public shape stays the same.

Likely impact:

- direct callers are semantically relevant
- importers may not need changes
- tests covering behavior may be relevant

#### `signature`

Meaning: parameter list, generics, return type, or callable contract changes.

Likely impact:

- all direct callers
- transitive wrappers
- exported API consumers

#### `rename`

Meaning: symbol/file/module name changes.

Likely impact:

- all references
- import sites
- re-exports
- callers

#### `delete`

Meaning: target removed entirely.

Likely impact:

- all references fail
- reverse dependencies fail
- transitive dependents may need redesign

#### `visibility`

Meaning: target becomes more or less accessible.

Likely impact:

- external package/module consumers
- re-export graph

#### `side-effect`

Meaning: file-level execution or initialization behavior changes.

Likely impact:

- importers of the file
- transitive importers if initialization order matters

### Impact algorithm rules

1. Resolve the target node.
2. Select graph traversals based on `change_type`.
3. Walk file edges, symbol edges, or both.
4. Record every impacted node with:
   - reason
   - path or symbol name
   - distance
   - certainty
5. Group results into high-confidence and uncertain output.

---

## 11. System architecture

### High-level architecture

`scope` should be built as a layered Rust project:

1. **Core library**
   - indexing
   - parsing
   - extraction
   - resolution
   - query engine
   - SQLite storage

2. **CLI**
   - command parsing
   - human-readable output
   - JSON formatting

3. **Optional MCP/agent wrapper**
   - exposes the same query engine to coding agents

### Recommended modules

- `scanner` - file discovery and ignore handling
- `adapters` - per-language tree-sitter wrappers
- `extractor` - imports, symbols, exports, calls
- `resolver` - symbol and import resolution
- `store` - SQLite schema and persistence
- `graph` - traversal and impact logic
- `query` - user-facing query handlers
- `output` - human and JSON renderers
- `config` - repo root, language, db path, settings
- `bench` - benchmarking utilities
- `mcp` - future integration layer

### Repository structure

```text
scope/
  Cargo.toml
  crates/
    scope-core/
    scope-cli/
    scope-mcp/
  fixtures/
    rust_small/
    rust_medium/
    ts_small/
  docs/
    plan.md
```

### Why separate crates

- `scope-core` can be embedded later in other tools.
- `scope-cli` stays thin and focused.
- `scope-mcp` can evolve independently without polluting the core.

---

## 12. Technology choices

### Language

Rust

### Recommended crates

- `clap` for CLI
- `ignore` and `walkdir` for repo traversal
- `tree-sitter` and language grammars for parsing
- `rusqlite` for persistence
- `serde` and `serde_json` for output
- `rayon` for parallel indexing
- `blake3` for content hashing
- `notify` for watch mode later
- `thiserror` and `anyhow` for errors
- `tracing` and `tracing-subscriber` for logs

### Why SQLite

- local and embeddable
- fast enough for graph lookups
- easy to inspect during development
- durable across sessions
- good fit for indexing metadata and edges

---

## 13. Extraction model

### What the extractor should capture

For each file:

- file path
- language
- imports/re-exports
- top-level symbols
- symbol visibility
- exported symbols
- direct calls inside symbol bodies
- line/byte spans
- parse errors or unsupported constructs

### Symbol kinds for v1

- function
- method
- struct/class
- enum/type alias
- module/namespace
- constant/static

### Visibility normalization

Use one shared enum across languages:

- `local`
- `module`
- `package`
- `public`
- `unknown`

### Rules for v1

- prioritize top-level definitions
- collect nested definitions only when cheap and reliable
- capture direct static call sites only
- ignore unresolved dynamic dispatch unless adapter can flag it as uncertain

---

## 14. Resolution model

### Import resolution

Map raw import syntax to a repository file when possible.

Possible outcomes:

- exact local file match
- module match
- unresolved external dependency
- unresolved dynamic import

### Symbol resolution

Map a call site or reference to a known in-repo symbol.

Resolution strategies:

1. lexical/module scope lookup
2. explicit import mapping
3. same-file symbol lookup
4. language-specific namespace rules
5. fallback to unresolved/heuristic

### Important constraint

v1 should only claim `resolved` when the adapter has enough evidence. It is better to miss some links than to invent false edges with high confidence.

---

## 15. Persistence design

### Core tables

```sql
CREATE TABLE files (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  language TEXT NOT NULL,
  hash TEXT NOT NULL,
  mtime INTEGER,
  parse_status TEXT NOT NULL
);

CREATE TABLE symbols (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL,
  qualname TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  visibility TEXT NOT NULL,
  exported INTEGER NOT NULL,
  span_start INTEGER,
  span_end INTEGER,
  FOREIGN KEY(file_id) REFERENCES files(id)
);

CREATE TABLE imports (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL,
  raw_text TEXT NOT NULL,
  resolved_file_id INTEGER,
  span_start INTEGER,
  span_end INTEGER,
  certainty TEXT NOT NULL,
  FOREIGN KEY(file_id) REFERENCES files(id),
  FOREIGN KEY(resolved_file_id) REFERENCES files(id)
);

CREATE TABLE file_edges (
  id INTEGER PRIMARY KEY,
  from_file_id INTEGER NOT NULL,
  to_file_id INTEGER NOT NULL,
  kind TEXT NOT NULL,
  certainty TEXT NOT NULL,
  FOREIGN KEY(from_file_id) REFERENCES files(id),
  FOREIGN KEY(to_file_id) REFERENCES files(id)
);

CREATE TABLE symbol_edges (
  id INTEGER PRIMARY KEY,
  from_symbol_id INTEGER NOT NULL,
  to_symbol_id INTEGER NOT NULL,
  kind TEXT NOT NULL,
  certainty TEXT NOT NULL,
  FOREIGN KEY(from_symbol_id) REFERENCES symbols(id),
  FOREIGN KEY(to_symbol_id) REFERENCES symbols(id)
);

CREATE TABLE index_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

### Required indices

```sql
CREATE INDEX idx_files_path ON files(path);
CREATE INDEX idx_symbols_qualname ON symbols(qualname);
CREATE INDEX idx_symbols_file_id ON symbols(file_id);
CREATE INDEX idx_file_edges_from ON file_edges(from_file_id);
CREATE INDEX idx_file_edges_to ON file_edges(to_file_id);
CREATE INDEX idx_symbol_edges_from ON symbol_edges(from_symbol_id);
CREATE INDEX idx_symbol_edges_to ON symbol_edges(to_symbol_id);
```

### Storage rules

- Always store normalized paths relative to repo root.
- Always store content hash for incremental indexing.
- Always delete and rebuild stale edges for changed files.

---

## 16. Indexing pipeline

### Full index pipeline

1. Detect repo root.
2. Load config and DB.
3. Walk files respecting `.gitignore` and adapter-supported extensions.
4. Hash file content.
5. Skip unchanged files where possible.
6. Parse changed files with tree-sitter.
7. Extract normalized entities and edges.
8. Resolve imports and symbol calls.
9. Write nodes and edges to SQLite.
10. Store index metadata and timing metrics.

### Incremental index pipeline

1. Compare stored hash with current hash.
2. For changed files:
   - delete old symbols/imports/edges for that file
   - re-parse and re-extract
   - re-resolve affected edges
3. Optionally re-evaluate reverse edges for dependent files if needed by language rules.

### Watch mode later

Use file notifications to trigger targeted re-indexing.

---

## 17. Query engine design

### Query categories

1. File graph
   - dependencies
   - reverse dependencies
   - transitive imports

2. Symbol inventory
   - symbols in file
   - public surface
   - visibility checks

3. Call graph
   - callees of symbol
   - callers of symbol
   - transitive caller/callee chain

4. Impact analysis
   - blast radius for change type
   - grouped by confidence
   - explainable reasons

### Explainability requirement

Every result in `impact`, `callers`, and `deps --reverse` should be explainable.

Minimum explanation fields:

- source node
- target node
- edge kind
- reason string
- certainty
- traversal distance

---

## 18. Milestones

## Milestone 0 - Foundation

### Goal

Set up project structure, core abstractions, test fixtures, and CLI skeleton.

### Deliverables

- Cargo workspace
- `scope-core` and `scope-cli`
- config handling
- logging setup
- base DB migration support
- fixture repos for tests
- command stubs with empty JSON output

### Acceptance criteria

- `scope --help` works
- `scope index`, `deps`, `symbols`, `calls`, `impact` commands exist
- test fixtures can be loaded in integration tests

### Tasks

- initialize workspace
- define shared data model types
- define JSON response envelopes
- implement basic error handling and tracing

---

## Milestone 1 - File dependency graph

### Goal

Answer file import and reverse-import questions reliably.

### Deliverables

- repo scanner respecting `.gitignore`
- Rust adapter for file-level imports/modules
- file nodes and file edges in SQLite
- `scope index`
- `scope deps <file>`
- `scope deps <file> --reverse`

### Acceptance criteria

- given a fixture repo, `deps` returns the exact imported local files for supported patterns
- `--reverse` returns all in-repo files that depend on the target file
- JSON output is stable and tested

### Tasks

- build file scanner
- create adapter trait
- implement Rust import extraction
- persist file graph
- add integration tests and golden JSON

---

## Milestone 2 - Symbol inventory

### Goal

Expose public/internal surface at the file and module level.

### Deliverables

- top-level symbol extraction
- visibility normalization
- export detection
- `scope symbols <file>`

### Acceptance criteria

- symbols in a test fixture match expected name, kind, visibility, and spans
- exported/public symbols are correctly labeled for supported Rust patterns

### Tasks

- extract functions, structs, enums, modules, constants
- map Rust visibility into normalized enum
- persist symbols and spans
- add fixture assertions

---

## Milestone 3 - Direct call graph

### Goal

Answer “who calls this?” and “what does this call?” for resolvable in-repo symbols.

### Deliverables

- call-site extraction
- same-file and imported symbol resolution
- `scope calls <symbol>`
- `scope callers <symbol>`

### Acceptance criteria

- direct callers/callees in fixture repos are returned with correct certainty
- unresolved calls are either omitted or labeled uncertain, never mislabeled as exact

### Tasks

- extract direct function call sites
- resolve same-module references
- resolve imported symbol references
- persist symbol edges
- add reason strings for query output

---

## Milestone 4 - Impact engine

### Goal

Provide useful blast-radius analysis by change type.

### Deliverables

- `scope impact <target> --change-type <type>`
- change-type aware traversal rules
- grouped confidence output
- explain reasons for every impacted node

### Acceptance criteria

- rename/delete/signature/body queries return expected impacted nodes for fixture repos
- output includes distance, reason, and certainty
- transitive traversal can be limited by depth

### Tasks

- encode impact rules per change type
- implement graph traversal engine
- add result grouping and summaries
- add high-confidence vs uncertain sections

---

## Milestone 5 - Incremental indexing

### Goal

Make re-indexing cheap enough for normal development flow.

### Deliverables

- content hash storage
- changed-file detection
- stale-edge invalidation
- incremental `scope index`

### Acceptance criteria

- unchanged files are skipped
- single-file edits only re-parse that file and refresh relevant edges
- benchmark shows material speedup vs full re-index on medium fixture repo

### Tasks

- add hashing and file metadata tracking
- add selective delete/reinsert logic
- benchmark full vs incremental indexing

---

## Milestone 6 - Agent integration

### Goal

Make `scope` easy for Claude Code or other agents to call.

### Deliverables

- finalized JSON contracts
- compact response mode
- optional stdio/MCP wrapper
- docs for agent usage patterns

### Acceptance criteria

- agent can query deps/symbols/calls/impact without reading files directly
- schema is stable enough for tool integration

### Tasks

- define machine-first JSON defaults
- build thin integration layer over `scope-core`
- write usage documentation and examples

---

## 19. Testing strategy

### Test categories

#### Unit tests

Use for:

- path normalization
- visibility mapping
- change-type rule selection
- simple resolution helpers

#### Fixture integration tests

Use small real repos to verify:

- file import graph
- symbol extraction
- call graph
- impact traversal

#### Golden JSON tests

Store expected JSON results for commands and compare output exactly.

#### Performance tests

Measure:

- full index time
- incremental index time
- query latency
- DB size

### Test repo fixtures

Create fixtures with:

- straightforward imports
- re-exports
- public/private functions
- nested modules
- ambiguous or unresolved calls
- one repo with deliberate dynamic limitations

### Reliability rule

Any unsupported construct should fail soft. The tool should still produce partial results and label uncertainty.

---

## 20. Performance targets

### Internal targets

Use these as engineering goals, not marketing claims until measured.

- warm dependency query: low milliseconds
- warm symbol query: low milliseconds
- warm impact query on medium repo: sub-100ms target
- incremental re-index after single-file edit: near-instant for normal repos

### Benchmark dimensions

Measure by:

- number of files
- lines of code
- language mix
- cold vs warm query
- full vs incremental index

### Benchmark command

```bash
scope benchmark --repo-root <path> --json
```

It should report:

- scan count
- parse time
- resolve time
- DB write time
- total index time
- query latency percentiles

---

## 21. Risks and mitigations

### Risk 1 - Overpromising “full impact”

**Problem:** users interpret impact as guaranteed runtime correctness.

**Mitigation:**
- position output as static blast radius
- include certainty labels
- document known blind spots

### Risk 2 - Language complexity explosion

**Problem:** every language has unique rules and edge cases.

**Mitigation:**
- use adapter architecture
- support one language family at a time
- keep shared model narrow and stable

### Risk 3 - False-positive call edges

**Problem:** aggressive resolution creates misleading impact data.

**Mitigation:**
- be conservative
- prefer missing a low-confidence edge over inventing one
- label heuristics explicitly

### Risk 4 - Slow re-indexing in large repos

**Problem:** a fast query engine is not enough if indexing is painful.

**Mitigation:**
- content hashing
- parallel parsing
- incremental invalidation
- benchmark early

### Risk 5 - Dynamic language blind spots

**Problem:** JS/TS and other languages may contain runtime-dependent imports and dispatch.

**Mitigation:**
- mark dynamic edges as uncertain
- keep them separate in output
- add framework-aware adapters later

---

## 22. Implementation roadmap

### Phase 1 - Weeks 1-2

- Milestone 0 foundation
- Milestone 1 file graph
- fixture repos and JSON tests

### Phase 2 - Weeks 3-4

- Milestone 2 symbol inventory
- Milestone 3 direct call graph
- improve resolver quality

### Phase 3 - Weeks 5-6

- Milestone 4 impact engine
- benchmark command
- stabilize JSON schema

### Phase 4 - Weeks 7+

- Milestone 5 incremental indexing
- Milestone 6 agent integration
- docs, packaging, release prep

This schedule should be treated as directional. Quality of the graph and schema stability matter more than hitting calendar dates.

---

## 23. Definition of done for v1

`scope` v1 is done when all of the following are true:

1. A user can index a Rust repo locally.
2. A user can query file deps, reverse deps, symbols, callers/callees, and impact.
3. All commands support `--json`.
4. Output includes reasons and certainty where relevant.
5. Incremental indexing works.
6. Fixture repos and golden JSON tests cover major supported patterns.
7. Documentation clearly states supported patterns and limitations.

---

## 24. First execution checklist

### Foundation checklist

- [ ] create Cargo workspace
- [ ] create `scope-core` crate
- [ ] create `scope-cli` crate
- [ ] add logging and config
- [ ] add SQLite migration layer
- [ ] define normalized data model
- [ ] define JSON response envelopes

### Indexing checklist

- [ ] implement repo scanner with ignore support
- [ ] implement Rust adapter skeleton
- [ ] parse files with tree-sitter
- [ ] extract imports/modules
- [ ] persist file nodes and edges
- [ ] add `scope index`

### Query checklist

- [ ] add `deps`
- [ ] add `deps --reverse`
- [ ] add `symbols`
- [ ] add `calls`
- [ ] add `callers`
- [ ] add `impact`
- [ ] add `explain`

### Quality checklist

- [ ] create small fixture repos
- [ ] add golden JSON tests
- [ ] add benchmark command
- [ ] add incremental re-indexing
- [ ] write limitations doc

---

## 25. Open questions

These should be resolved before or during Milestone 1:

1. Should v1 support only Rust, or Rust plus TS/JS at launch?
2. Should methods and trait dispatch be included in Rust v1, or postponed until after plain function calls are solid?
3. Should unresolved dynamic edges appear in default output, or only behind `--verbose`?
4. Should `scope` prefer a single SQLite DB per repo root, or allow workspace-level overrides?
5. Should the agent wrapper be built immediately after JSON stabilization or only after CLI adoption?

---

## 26. Recommended next steps

1. Freeze the CLI and JSON contract before writing extraction code.
2. Build the smallest end-to-end slice: scan -> parse -> persist -> `deps` query.
3. Add fixtures before adding more language features.
4. Only expand to symbol and impact analysis after file graph quality is proven.
5. Keep marketing language honest: “static blast radius” beats “guaranteed full impact.”

---

## 27. One-sentence summary

`scope` is a local Rust static-analysis engine that indexes repo files, symbols, and resolvable calls into SQLite so developers and coding agents can ask dependency and blast-radius questions instantly with structured, explainable results.
