+++
id = "impact-review"
title = "Impact Review"
summary = "Assemble impact, explanation, and context evidence for a planned code change."
tags = ["impact", "explain", "context", "report"]

[[arguments]]
name = "target"
description = "File path or symbol qualname to change."
required = true

[[arguments]]
name = "change_type"
description = "Change kind to analyze."
default = "body"

[[steps]]
title = "Estimate static impact"
command = "scope impact {{target}} --change-type {{change_type}}"
rationale = "Produces the first pass blast-radius estimate."

[[steps]]
title = "Explain why key files appear"
command = "scope explain {{target}} --depth 3"
rationale = "Connects impact results back to concrete graph edges."

[[steps]]
title = "Build the ranked read set"
command = "scope context --target {{target}} --change-type {{change_type}} --budget 600"
rationale = "Focuses review on the files that matter most for a safe change."

[[steps]]
title = "Capture the repo health baseline"
command = "scope report --json"
rationale = "Records the current health state before edits start."
+++
# Impact Review

Use this workflow before any non-trivial refactor, rename, signature change, or
behavioral edit. It turns a vague request into a concrete checklist of structural
evidence plus a before-change baseline.
