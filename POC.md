# POC: scope

> Proof of concept demonstrating real-time dependency graph extraction
> from source files using tree-sitter, with instant query response.

---

## Status

This Node.js proof of concept is currently **deferred / supplemental rather than the main execution path**, but the repository now includes a runnable prototype under `poc/` that was built to satisfy the Phase 0 validation bead.

It remains a time-boxed validation artifact rather than the primary product direction. The active implementation path for this project is still the Rust workspace described in `PLAN.md` and implemented under `crates/`.

What now exists:
- `poc/index.js` — a runnable Node.js script using tree-sitter and SQLite
- `poc/package.json` — local dependencies and helper scripts
- `.scope/poc.db` — SQLite output created by the prototype at runtime
- validation against `fixtures/ts_small`

Current validated behavior:
- indexes `.js/.jsx/.ts/.tsx` files by walking the tree directly and skipping `.git`, `.scope`, `node_modules`, and `target`
- parses JavaScript with `tree-sitter-javascript` and TypeScript with `tree-sitter-typescript`
- persists files, imports, exports, functions, and direct call names into SQLite
- resolves relative imports and re-export barrels well enough to make `impact` traversal useful on the fixture graph
- answers JSON queries for `index`, `file`, `fn`, and `impact`

Current limitations observed during validation:
- no `.gitignore` integration yet; ignore handling is only a fixed directory skip list
- export extraction is intentionally shallow and limited to common direct export / re-export patterns
- call edges store raw callee text only and do not resolve imported symbols back to definitions
- command coverage stayed narrower than the original phase-0 sketch: the script exposes `index`, `file`, `fn`, and `impact`, but not standalone `deps`, `callers`, or `why`
- no certainty labels, no `why` query, and no Dijkstra/path explanation yet
- not yet validated on a real external open-source project such as Express or Fastify

Latest validation snapshot against `fixtures/ts_small`:
- `node index.js index ../fixtures/ts_small` currently reports **7 files**, **7 imports**, **12 exports**, **5 functions**, and **5 calls**
- `node index.js file ../fixtures/ts_small/src/auth/jwt.ts` shows reverse edges from both `auth/middleware.ts` and the alias re-export in `auth/aliases.ts`
- `node index.js impact ../fixtures/ts_small/src/auth/jwt.ts` reports **4 affected files** total, proving the traversal carries through both the alias re-export and the `auth/index.ts` barrel to `src/index.ts`

If this POC is extended later, it should remain an explicit, time-boxed validation track rather than an implicit parallel roadmap.

## POC Goal

Prove that we can:
1. Parse JavaScript/TypeScript imports and exports using tree-sitter
2. Build a usable dependency graph in memory
3. Answer "who imports X" and "what does X import" instantly
4. Output structured JSON that Claude Code can consume

---

## POC Scope

Written in **Node.js** for fast iteration.  
Production will be Rust + petgraph.

---

## Setup

```bash
cd poc
npm install
```

**Dependencies:**
```json
{
  "tree-sitter": "^0.21.1",
  "tree-sitter-javascript": "^0.21.4",
  "tree-sitter-typescript": "^0.21.2",
  "better-sqlite3": "^12.4.1"
}
```

These versions were chosen because the initial attempt to mix `tree-sitter` 0.25 with `tree-sitter-typescript` 0.23 produced a peer-dependency conflict during `npm install`.

---

## POC Code

### `poc/index.js`

The implementation now lives directly in the repository at `poc/index.js`. Rather than duplicating the full source in this document, the important validated design points are:

- uses separate parsers for JavaScript and TypeScript
- writes to `.scope/poc.db`
- creates SQLite tables for `files`, `imports`, `exports`, `functions`, and `calls`
- resolves relative imports across `.ts/.tsx/.js/.jsx` plus `index.*` barrel files
- treats both `import` statements and re-export statements as dependency edges for traversal
- exposes JSON commands: `index`, `file`, `fn`, `impact`

The current code is intentionally small and conservative. It proves the extraction/storage/query loop, but it is not meant to be a production architecture.

---

## Run the POC

```bash
cd poc
npm install

# Index the repository's TS fixture
node index.js index ../fixtures/ts_small

# Query a file
node index.js file ../fixtures/ts_small/src/auth/jwt.ts

# Query a function
node index.js fn verifyToken

# Impact analysis
node index.js impact ../fixtures/ts_small/src/auth/jwt.ts
```

---

## Test Fixture Used For Validation

The current validation pass uses `fixtures/ts_small`, which already exists in the Rust workspace as a small TypeScript dependency graph.

Relevant files:

### `fixtures/ts_small/src/auth/jwt.ts`
```ts
export function sign(payload: string): string {
  return `signed:${payload}`;
}

export function verify(token: string): boolean {
  return token.startsWith("signed:");
}
```

### `fixtures/ts_small/src/auth/middleware.ts`
```ts
import { verify } from "./jwt";

export function verifyToken(token: string): boolean {
  return verify(token);
}
```

### `fixtures/ts_small/src/auth/index.ts`
```ts
export { verifyToken } from "./middleware";
```

### `fixtures/ts_small/src/index.ts`
```ts
export { verifyToken } from "./auth/index";
export { format } from "./utils/formatter";
```

---

## Example Validated Output

```bash
$ cd poc
$ node index.js index ../fixtures/ts_small

{
  "command": "index",
  "status": "ok",
  "data": {
    "target": "fixtures/ts_small",
    "database": ".scope/poc.db",
    "stats": {
      "files": 7,
      "imports": 7,
      "exports": 12,
      "functions": 5,
      "calls": 5
    }
  }
}

$ node index.js impact ../fixtures/ts_small/src/auth/jwt.ts

{
  "command": "impact",
  "status": "ok",
  "data": {
    "file": "fixtures/ts_small/src/auth/jwt.ts",
    "direct_dependents": [
      "fixtures/ts_small/src/auth/aliases.ts",
      "fixtures/ts_small/src/auth/middleware.ts"
    ],
    "transitive_dependents": [
      "fixtures/ts_small/src/auth/index.ts",
      "fixtures/ts_small/src/index.ts"
    ],
    "total_affected": 4,
    "risk": "MEDIUM"
  }
}

$ node index.js fn verifyToken

{
  "command": "fn",
  "status": "ok",
  "data": {
    "function": "verifyToken",
    "defined_in": [
      {
        "file": "fixtures/ts_small/src/auth/middleware.ts",
        "start_line": 3
      }
    ],
    "called_by": [],
    "calls": [
      {
        "to_fn": "verify",
        "line": 4
      }
    ]
  }
}
```

---

## Why This Is Valuable for Claude Code

When Claude Code needs to refactor a function, it currently:
1. Reads the file (1 read)
2. Sees imports, reads each imported file (3–5 reads)
3. Tries to guess who imports THIS file (blind)
4. Makes changes without knowing full blast radius

With `scope`:
1. Runs `scope impact <file>` → instant JSON
2. Knows exactly which 12 files depend on this
3. Reads only the relevant ones
4. Makes changes with confidence

**Result: 70–80% fewer file reads per task.**

---

## Findings That Should Inform The Rust Path

1. Modeling re-export statements as dependency edges matters immediately; without that, impact traversal misses barrel chains.
2. Alias re-exports matter too: in the current fixture, `auth/aliases.ts` is a direct dependent of `auth/jwt.ts`, so blast-radius traversal must treat `export { verify as verifyJwt, sign as signJwt } from "./jwt"` as a first-class edge.
3. A tiny SQLite-backed prototype is enough to validate useful query semantics before designing a richer graph layer.
4. Call-site extraction is easy to sketch, but symbol resolution remains the real difficulty; storing raw callee text is not enough for trustworthy callers/callees results.
5. Version compatibility across tree-sitter packages is a practical risk that should be pinned carefully in any future JS/TS validation work.
6. Fixed-directory ignore rules are not enough for a serious prototype; `.gitignore`-aware walking should be part of any next expansion.

## Gap Against The Original Phase-0 Sketch

Compared with the original Phase 0 checklist in `PLAN.md`, the current POC proved the most important loop — tree-sitter extraction into SQLite with useful `impact` answers on a controlled fixture — but it intentionally stopped short of full temporary feature parity.

What the artifact validates well:
- parser setup and language split for JS vs TS
- relative-import plus `index.*` barrel resolution
- dependency traversal over direct imports and re-exports
- JSON output that an agent can consume immediately

What remains intentionally unproven in this Node track:
- `.gitignore`-aware walking
- certainty labels and reason trails
- standalone `deps`, `callers`, and `why` query surfaces
- validation on a real external OSS repository

That gap is useful signal, not failure: it shows the riskiest value was confirming dependency-edge extraction and traversal behavior, while richer query semantics and production hardening still belonged on the Rust path.

## Next Steps (Production Rust)

1. Carry the re-export edge lesson into the Rust adapter and future TS/JS adapter design
2. Keep SQLite as the durable source of truth even if an in-memory graph layer is added for traversal speed
3. Add certainty labels and reason trails before trusting cross-file call answers
4. Add `.gitignore`-aware walking if the POC is ever extended beyond the fixture repo
5. Treat this prototype as validation input, not as code to incrementally evolve into production
