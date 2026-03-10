use std::{collections::HashSet, fs, path::Path};

use glob::Pattern;

use crate::{
    ArchCheckResult, ArchConfig, ArchFileEdge, ArchLayer, ArchRule, ArchViolation, RepoPath,
    ScopeError, ScopeResult, Store,
};

pub fn load_arch_config(repo_root: &Path) -> ScopeResult<ArchConfig> {
    let config_path = repo_root.join(".scope/arch.toml");
    let source =
        fs::read_to_string(&config_path).map_err(|error| ScopeError::io(&config_path, error))?;
    let config: ArchConfig = toml::from_str(&source)
        .map_err(|error| ScopeError::InvalidInput(format!("invalid arch config: {error}")))?;
    validate_arch_config(&config)?;
    Ok(config)
}

pub fn arch_check(store: &Store, config: &ArchConfig) -> ScopeResult<ArchCheckResult> {
    let mut violations = Vec::new();
    let edges = store.query_file_edges()?;
    let mut checked_layered_edges = 0;

    for edge in &edges {
        let Some(from_layer) = resolve_layer(&config.layers, &edge.from_file)? else {
            continue;
        };
        let Some(to_layer) = resolve_layer(&config.layers, &edge.to_file)? else {
            continue;
        };
        checked_layered_edges += 1;

        for rule in config
            .rules
            .iter()
            .filter(|rule| rule.from == from_layer.name)
        {
            if rule
                .may_not_import
                .iter()
                .any(|target| target == &to_layer.name)
            {
                violations.push(ArchViolation {
                    from_file: edge.from_file.clone(),
                    to_file: edge.to_file.clone(),
                    from_layer: from_layer.name.clone(),
                    to_layer: to_layer.name.clone(),
                    message: rule.message.clone().unwrap_or_else(|| {
                        format!("{} may not import {}", rule.from, to_layer.name)
                    }),
                });
            }
        }
    }

    violations.sort_by(|left, right| {
        left.from_file
            .0
            .cmp(&right.from_file.0)
            .then(left.to_file.0.cmp(&right.to_file.0))
            .then(left.from_layer.cmp(&right.from_layer))
            .then(left.to_layer.cmp(&right.to_layer))
            .then(left.message.cmp(&right.message))
    });

    Ok(ArchCheckResult {
        config_path: RepoPath::from(".scope/arch.toml"),
        checked_edges: edges.len(),
        checked_layered_edges,
        violations,
    })
}

fn validate_arch_config(config: &ArchConfig) -> ScopeResult<()> {
    if config.layers.is_empty() {
        return Err(ScopeError::InvalidInput(
            "arch config must define at least one [[layer]]".to_string(),
        ));
    }

    let mut names = HashSet::new();
    for layer in &config.layers {
        if !names.insert(layer.name.as_str()) {
            return Err(ScopeError::InvalidInput(format!(
                "duplicate layer name: {}",
                layer.name
            )));
        }
        Pattern::new(&layer.pattern).map_err(|error| {
            ScopeError::InvalidInput(format!(
                "invalid layer pattern '{}': {error}",
                layer.pattern
            ))
        })?;
    }

    for rule in &config.rules {
        if !names.contains(rule.from.as_str()) {
            return Err(ScopeError::InvalidInput(format!(
                "rule references unknown layer: {}",
                rule.from
            )));
        }
        for target in &rule.may_not_import {
            if !names.contains(target.as_str()) {
                return Err(ScopeError::InvalidInput(format!(
                    "rule references unknown target layer: {}",
                    target
                )));
            }
        }
    }

    Ok(())
}

fn resolve_layer<'a>(
    layers: &'a [ArchLayer],
    path: &RepoPath,
) -> ScopeResult<Option<&'a ArchLayer>> {
    for layer in layers {
        let pattern = Pattern::new(&layer.pattern).map_err(|error| {
            ScopeError::InvalidInput(format!(
                "invalid layer pattern '{}': {error}",
                layer.pattern
            ))
        })?;
        if pattern.matches(&path.0) {
            return Ok(Some(layer));
        }
    }
    Ok(None)
}

#[allow(dead_code)]
fn _edge(_edge: &ArchFileEdge, _rule: &ArchRule) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_unknown_rule_layer() {
        let config = ArchConfig {
            layers: vec![ArchLayer {
                name: "services".to_string(),
                pattern: "src/services/**".to_string(),
                description: None,
            }],
            rules: vec![ArchRule {
                from: "routes".to_string(),
                may_not_import: vec!["services".to_string()],
                message: None,
            }],
        };

        let error = validate_arch_config(&config).unwrap_err();
        assert!(error.to_string().contains("unknown layer"));
    }

    #[test]
    fn resolves_first_matching_layer() {
        let layers = vec![
            ArchLayer {
                name: "first".to_string(),
                pattern: "src/**".to_string(),
                description: None,
            },
            ArchLayer {
                name: "second".to_string(),
                pattern: "src/services/**".to_string(),
                description: None,
            },
        ];

        let layer = resolve_layer(&layers, &RepoPath::from("src/services/user.ts"))
            .unwrap()
            .unwrap();
        assert_eq!(layer.name, "first");
    }
}
