Fixture for dynamic import and unresolved-behavior scenarios.

- `src/index.js` uses static imports so the file graph still has ordinary edges.
- `src/computed_import.ts` uses `import(specifier)` and should remain partial.
- `src/dynamic_require.js` uses `require(name)` and should remain partial.

Coverage should verify that static edges remain queryable while unresolved dynamic patterns do not invent false dependency or call edges.
