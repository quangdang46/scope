+++
id = "dependency-trace"
title = "Dependency Trace"
summary = "Collect direct and transitive dependency evidence around a file or symbol before editing."
tags = ["deps", "callers", "context", "pack"]

[[arguments]]
name = "target"
description = "File path or symbol qualname to inspect."
required = true

[[arguments]]
name = "change_type"
description = "Change kind to use when gathering context evidence."
default = "body"

[[steps]]
title = "Map direct and transitive dependencies"
command = "scope deps {{target}} --transitive --depth 2"
rationale = "Shows structural coupling around the target before you change it."

[[steps]]
title = "Collect transitive callers"
command = "scope callers {{target}} --transitive"
rationale = "Highlights upstream symbols that may observe or depend on the target."

[[steps]]
title = "Build the minimum must-read context"
command = "scope context --target {{target}} --change-type {{change_type}} --budget 400"
rationale = "Ranks the smallest file set worth reading before editing."

[[steps]]
title = "Generate a plain-text handoff"
command = "scope pack {{target}} --change-type {{change_type}} --budget 400"
rationale = "Produces a compact prompt-friendly summary for the next agent loop."
+++
# Dependency Trace

Use this workflow when an agent needs to understand the blast radius around
`{{target}}` fast without improvising its own prompt. Run the steps in order and keep
the outputs together as evidence for code review or edit planning.
