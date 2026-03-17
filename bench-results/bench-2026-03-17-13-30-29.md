# scope benchmark results

Date: 2026-03-17 13:30:29
Commit: `4d08d43`
Build: current working tree
Command: `scope benchmark --fixture rust_small --iterations 1 --write-report`
Fixture: `rust_small`
Iterations: 1

## Current performance state

### Workload
- Indexed files: 5
- Mutation target: `src/parser.rs`
- Mutation kind: `append_comment`

### Phase summary
| Phase | Avg ms | Min ms | Max ms | Files processed | Changed files | Deleted files | Affected files |
|-------|--------|--------|--------|-----------------|---------------|---------------|----------------|
| Full re-index | 526 | 526 | 526 | 5 | 5 | 0 | 5 |
| Incremental re-index | 185 | 185 | 185 | 3 | 1 | 0 | 3 |

## Assessment

- Incremental indexing saved 341 ms versus a full re-index.
- Incremental indexing ran at 35% of the full indexing time.
- The benchmark uses an isolated repo copy and appends a small comment mutation before re-indexing.

## Update instructions

1. Re-run `scope benchmark --fixture rust_small --iterations 1 --write-report`.
2. Review the diff in `bench-results/benchmark.md`.
3. Keep the generated `bench-results/bench-YYYY-MM-DD-HH-MM-SS.md` snapshot if you want a dated artifact.

