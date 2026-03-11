Prepared git-history fixture scaffold for Milestone 12 cochange validation.

This fixture is intended to host a tiny repository with deterministic commit history for
`scope cochange` tests driven by real `git log` ingestion rather than manual `file_churn`
seeding.

The companion script `create_git_history.sh` materializes the fixture in a target directory.
It is intentionally separate from the committed fixture contents so tests can create isolated,
throwaway repos with known commit topology.
