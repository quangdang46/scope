use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{JsonEnvelope, ScopeError, ScopeResult};

const WORKFLOW_DIR: &str = ".scope/workflows";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowArgument {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStep {
    pub title: String,
    pub command: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSummary {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowDefinition {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub arguments: Vec<WorkflowArgument>,
    pub steps: Vec<WorkflowStep>,
    pub body: String,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowListResult {
    pub directory: String,
    pub workflows: Vec<WorkflowSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRenderResult {
    pub workflow: WorkflowDefinition,
    pub arguments: BTreeMap<String, String>,
    pub rendered_steps: Vec<WorkflowStep>,
    pub rendered_markdown: String,
}

#[derive(Debug, Deserialize)]
struct RawWorkflowFrontmatter {
    id: String,
    title: String,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    arguments: Vec<WorkflowArgument>,
    #[serde(default)]
    steps: Vec<WorkflowStep>,
}

pub fn workflow_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(WORKFLOW_DIR)
}

pub fn discover_workflows(repo_root: &Path) -> ScopeResult<Vec<WorkflowDefinition>> {
    let directory = workflow_dir(repo_root);
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| ScopeError::io(&directory, error))? {
        let entry = entry.map_err(|error| ScopeError::io(&directory, error))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            entries.push(path);
        }
    }
    entries.sort();

    let mut workflows = Vec::new();
    let mut seen_ids = BTreeMap::<String, PathBuf>::new();
    for path in entries {
        let workflow = parse_workflow(repo_root, &path)?;
        if let Some(existing) = seen_ids.insert(workflow.id.clone(), path.clone()) {
            return Err(ScopeError::ParseFailed {
                path: path.display().to_string(),
                message: format!(
                    "duplicate workflow id '{}' in {} and {}",
                    workflow.id,
                    existing.display(),
                    path.display()
                ),
            });
        }
        workflows.push(workflow);
    }

    Ok(workflows)
}

pub fn list_workflows(repo_root: &Path) -> ScopeResult<WorkflowListResult> {
    let workflows = discover_workflows(repo_root)?
        .into_iter()
        .map(|workflow| WorkflowSummary {
            id: workflow.id,
            title: workflow.title,
            summary: workflow.summary,
            tags: workflow.tags,
            source_path: workflow.source_path,
        })
        .collect();

    Ok(WorkflowListResult {
        directory: workflow_dir(repo_root).display().to_string(),
        workflows,
    })
}

pub fn render_workflow(
    repo_root: &Path,
    workflow_id: &str,
    provided_args: &BTreeMap<String, String>,
) -> ScopeResult<WorkflowRenderResult> {
    let workflow = discover_workflows(repo_root)?
        .into_iter()
        .find(|workflow| workflow.id == workflow_id)
        .ok_or_else(|| ScopeError::NotFound {
            kind: "workflow",
            value: workflow_id.to_string(),
        })?;

    let arguments = resolve_arguments(&workflow, provided_args)?;
    let rendered_steps = workflow
        .steps
        .iter()
        .map(|step| WorkflowStep {
            title: step.title.clone(),
            command: render_template(&step.command, &arguments),
            rationale: step
                .rationale
                .as_ref()
                .map(|text| render_template(text, &arguments)),
        })
        .collect();
    let rendered_markdown = render_template(&workflow.body, &arguments);

    Ok(WorkflowRenderResult {
        workflow,
        arguments,
        rendered_steps,
        rendered_markdown,
    })
}

pub fn workflow_list_envelope(result: WorkflowListResult) -> JsonEnvelope<WorkflowListResult> {
    JsonEnvelope::success("workflow_list", result)
}

pub fn workflow_show_envelope(result: WorkflowRenderResult) -> JsonEnvelope<WorkflowRenderResult> {
    JsonEnvelope::success("workflow_show", result)
}

fn parse_workflow(repo_root: &Path, path: &Path) -> ScopeResult<WorkflowDefinition> {
    let source = fs::read_to_string(path).map_err(|error| ScopeError::io(path, error))?;
    let normalized = source.replace("\r\n", "\n");
    let (frontmatter, body) =
        split_frontmatter(&normalized).ok_or_else(|| ScopeError::ParseFailed {
            path: path.display().to_string(),
            message: "workflow markdown must start with TOML frontmatter fenced by +++".to_string(),
        })?;
    let frontmatter: RawWorkflowFrontmatter =
        toml::from_str(frontmatter).map_err(|error| ScopeError::ParseFailed {
            path: path.display().to_string(),
            message: format!("invalid workflow frontmatter: {error}"),
        })?;

    if frontmatter.id.trim().is_empty() {
        return Err(ScopeError::ParseFailed {
            path: path.display().to_string(),
            message: "workflow id cannot be empty".to_string(),
        });
    }
    if frontmatter.steps.is_empty() {
        return Err(ScopeError::ParseFailed {
            path: path.display().to_string(),
            message: "workflow must declare at least one step".to_string(),
        });
    }

    let source_path = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string();

    Ok(WorkflowDefinition {
        id: frontmatter.id,
        title: frontmatter.title,
        summary: frontmatter.summary,
        tags: frontmatter.tags,
        arguments: frontmatter.arguments,
        steps: frontmatter.steps,
        body: body.trim().to_string(),
        source_path,
    })
}

fn split_frontmatter(source: &str) -> Option<(&str, &str)> {
    let rest = source.strip_prefix("+++\n")?;
    let end = rest.find("\n+++\n")?;
    let frontmatter = &rest[..end];
    let body = &rest[end + "\n+++\n".len()..];
    Some((frontmatter, body))
}

fn resolve_arguments(
    workflow: &WorkflowDefinition,
    provided_args: &BTreeMap<String, String>,
) -> ScopeResult<BTreeMap<String, String>> {
    let declared = workflow
        .arguments
        .iter()
        .map(|argument| argument.name.as_str())
        .collect::<Vec<_>>();
    for key in provided_args.keys() {
        if !declared.iter().any(|name| *name == key.as_str()) {
            return Err(ScopeError::InvalidInput(format!(
                "workflow '{}' does not declare argument '{}'",
                workflow.id, key
            )));
        }
    }

    let mut resolved = BTreeMap::new();
    for argument in &workflow.arguments {
        if let Some(default) = &argument.default {
            resolved.insert(argument.name.clone(), default.clone());
        }
    }
    for (key, value) in provided_args {
        resolved.insert(key.clone(), value.clone());
    }
    for argument in &workflow.arguments {
        if argument.required && !resolved.contains_key(&argument.name) {
            return Err(ScopeError::InvalidInput(format!(
                "workflow '{}' requires argument '{}'",
                workflow.id, argument.name
            )));
        }
    }

    Ok(resolved)
}

fn render_template(template: &str, arguments: &BTreeMap<String, String>) -> String {
    let mut rendered = template.to_string();
    for (key, value) in arguments {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scope-workflow-{prefix}-{nanos}"))
    }

    fn write_workflow(root: &Path, name: &str, body: &str) {
        let path = workflow_dir(root);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(name), body).unwrap();
    }

    #[test]
    fn discovers_and_lists_markdown_workflows() {
        let root = unique_temp_dir("discover");
        fs::create_dir_all(&root).unwrap();
        write_workflow(
            &root,
            "dependency-trace.md",
            r#"+++
id = "dependency-trace"
title = "Dependency Trace"
summary = "Trace direct and transitive structure around a target."
tags = ["deps", "context"]

[[arguments]]
name = "target"
description = "File or symbol to inspect."
required = true

[[steps]]
title = "Map dependencies"
command = "scope deps {{target}} --transitive --depth 2"
+++
# Dependency Trace

Run the dependency and context commands before editing `{{target}}`.
"#,
        );

        let listed = list_workflows(&root).unwrap();
        assert_eq!(listed.workflows.len(), 1);
        assert_eq!(listed.workflows[0].id, "dependency-trace");
        assert_eq!(
            listed.workflows[0].source_path,
            ".scope/workflows/dependency-trace.md"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renders_workflow_with_defaults_and_explicit_arguments() {
        let root = unique_temp_dir("render");
        fs::create_dir_all(&root).unwrap();
        write_workflow(
            &root,
            "impact-review.md",
            r#"+++
id = "impact-review"
title = "Impact Review"
summary = "Collect impact evidence for a pending change."

[[arguments]]
name = "target"
description = "Symbol or file to inspect."
required = true

[[arguments]]
name = "change_type"
description = "Change kind for impact/context queries."
default = "body"

[[steps]]
title = "Impact"
command = "scope impact {{target}} --change-type {{change_type}}"

[[steps]]
title = "Context"
command = "scope context --target {{target}} --change-type {{change_type}} --budget 400"
rationale = "Capture the must-read file set."
+++
# Impact Review

Use the rendered commands below before changing `{{target}}`.
"#,
        );

        let mut args = BTreeMap::new();
        args.insert("target".to_string(), "parser::parse".to_string());
        let rendered = render_workflow(&root, "impact-review", &args).unwrap();

        assert_eq!(
            rendered.arguments.get("change_type"),
            Some(&"body".to_string())
        );
        assert_eq!(rendered.rendered_steps.len(), 2);
        assert_eq!(
            rendered.rendered_steps[0].command,
            "scope impact parser::parse --change-type body"
        );
        assert!(rendered.rendered_markdown.contains("parser::parse"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_required_arguments() {
        let root = unique_temp_dir("required");
        fs::create_dir_all(&root).unwrap();
        write_workflow(
            &root,
            "architecture-snapshot.md",
            r#"+++
id = "architecture-snapshot"
title = "Architecture Snapshot"
summary = "Summarize architecture evidence."

[[arguments]]
name = "target"
description = "File or symbol to inspect."
required = true

[[steps]]
title = "Surface"
command = "scope surface {{target}}"
+++
# Architecture Snapshot
"#,
        );

        let error = render_workflow(&root, "architecture-snapshot", &BTreeMap::new())
            .expect_err("missing target should fail");
        assert!(error.to_string().contains("requires argument 'target'"));

        fs::remove_dir_all(root).unwrap();
    }
}
