+++
id = "architecture-snapshot"
title = "Architecture Snapshot"
summary = "Gather architecture, surface, and risk evidence for a subsystem or file."
tags = ["surface", "arch", "risk", "stability"]

[[arguments]]
name = "target"
description = "File path or symbol qualname to summarize."
required = true

[[steps]]
title = "Inspect the public surface"
command = "scope surface {{target}}"
rationale = "Shows the API surface and exported symbols that shape downstream coupling."

[[steps]]
title = "Explain architecture rules"
command = "scope arch explain {{target}}"
rationale = "Pulls the active layer rules or capability boundaries into the review."

[[steps]]
title = "Check stability hotspots"
command = "scope stability --file {{target}}"
rationale = "Surfaces fan-in/fan-out pressure around the target."

[[steps]]
title = "Check churn-weighted risk"
command = "scope risk --file {{target}}"
rationale = "Adds historical volatility to the architecture snapshot."
+++
# Architecture Snapshot

Use this workflow when an agent needs a subsystem-level summary for design review,
handoff notes, or architectural documentation updates.
