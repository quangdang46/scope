#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <target-dir>" >&2
  exit 1
fi

target_dir="$1"
rm -rf "$target_dir"
mkdir -p "$target_dir/src"

cd "$target_dir"
git init -q

git config user.name "Scope Fixture"
git config user.email "fixture@example.com"

cat > src/parser.rs <<'EOF'
pub fn parse(input: &str) -> Vec<&str> {
    input.split(',').collect()
}
EOF

cat > src/utils.rs <<'EOF'
pub fn trim(input: &str) -> &str {
    input.trim()
}
EOF

cat > src/resolver.rs <<'EOF'
pub fn resolve(input: &str) -> String {
    input.to_string()
}
EOF

cat > Cargo.toml <<'EOF'
[package]
name = "cochange_fixture"
version = "0.1.0"
edition = "2021"
EOF

git add .
GIT_AUTHOR_DATE="2024-01-01T00:00:00Z" GIT_COMMITTER_DATE="2024-01-01T00:00:00Z" git commit -q -m "initial fixture"

printf '\n// commit c1\n' >> src/parser.rs
printf '\n// commit c1\n' >> src/utils.rs
git add src/parser.rs src/utils.rs
GIT_AUTHOR_DATE="2024-01-02T00:00:00Z" GIT_COMMITTER_DATE="2024-01-02T00:00:00Z" git commit -q -m "parser and utils evolve together"

printf '\n// commit c2\n' >> src/parser.rs
printf '\n// commit c2\n' >> src/utils.rs
printf '\n// commit c2\n' >> src/resolver.rs
git add src/parser.rs src/utils.rs src/resolver.rs
GIT_AUTHOR_DATE="2024-01-03T00:00:00Z" GIT_COMMITTER_DATE="2024-01-03T00:00:00Z" git commit -q -m "parser utils and resolver evolve together"

printf '\n// commit c3\n' >> src/parser.rs
git add src/parser.rs
GIT_AUTHOR_DATE="2024-01-04T00:00:00Z" GIT_COMMITTER_DATE="2024-01-04T00:00:00Z" git commit -q -m "parser evolves alone"

printf '\n// commit c4\n' >> src/resolver.rs
git add src/resolver.rs
GIT_AUTHOR_DATE="2024-01-05T00:00:00Z" GIT_COMMITTER_DATE="2024-01-05T00:00:00Z" git commit -q -m "resolver evolves alone"

echo "created cochange fixture repo at $target_dir"
