+++
id = "diff-investigation"
title = "Diff Investigation"
summary = "Triangulate changed files against blast radius, entry reachability, and quality gates."
tags = ["diff", "entry", "gate", "snapshot"]

[[arguments]]
name = "branch"
description = "Git branch or ref to compare against."
required = true

[[arguments]]
name = "compare_snapshot"
description = "Saved snapshot name for report/gate comparison."
default = "baseline"

[[steps]]
title = "List files affected by the branch diff"
command = "scope diff {{branch}}"
rationale = "Establishes the initial candidate set."

[[steps]]
title = "Check entry-point reachability"
command = "scope entry unreachable"
rationale = "Flags orphaned or suspicious files after the change."

[[steps]]
title = "Generate a comparative report"
command = "scope report --compare {{compare_snapshot}} --json"
rationale = "Compares the current graph to a stored baseline snapshot."

[[steps]]
title = "Evaluate configured gates"
command = "scope gate --compare {{compare_snapshot}}"
rationale = "Answers whether the diff violates any policy thresholds."
+++
# Diff Investigation

Use this workflow during review or release prep when the question is "what changed and
how risky is it?" rather than "how do I edit this file?".
