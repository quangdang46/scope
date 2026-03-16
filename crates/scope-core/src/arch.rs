use std::{collections::HashSet, fs, path::Path};

use glob::Pattern;
use serde::Serialize;

use crate::{
    model::{ArchExplainImport, ArchExplainRule},
    ArchCheckResult, ArchConfig, ArchExplainResult, ArchFileEdge, ArchInitResult, ArchLayer,
    ArchRule, ArchViolation, GateConfig, GateMetric, RepoPath, ScopeError, ScopeResult, Store,
    TestConfig,
};

pub const ARCH_INIT_MESSAGE: &str =
    "Generated .scope/arch.toml — review and customize before using in CI";

const ARCH_INIT_MAX_DEPTH: usize = 2;
const ARCH_INIT_SKIP_DIRS: &[&str] = &[".git", ".scope", "node_modules", "target"];
const ARCH_INIT_LAYER_ALIASES: &[(&str, &[&str])] = &[
    ("routes", &["routes", "controllers", "views"]),
    ("middleware", &["middleware"]),
    ("services", &["services", "use-cases"]),
    ("models", &["models", "entities", "domain"]),
    ("utils", &["utils", "helpers", "lib"]),
    ("config", &["config"]),
];

#[derive(Debug, Serialize)]
struct StarterArchConfig {
    #[serde(rename = "layer", skip_serializing_if = "Vec::is_empty")]
    layers: Vec<ArchLayer>,
    #[serde(rename = "rule", skip_serializing_if = "Vec::is_empty")]
    rules: Vec<ArchRule>,
}

pub fn load_arch_config(repo_root: &Path) -> ScopeResult<ArchConfig> {
    let config_path = repo_root.join(".scope/arch.toml");
    let source = match fs::read_to_string(&config_path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ArchConfig::default())
        }
        Err(error) => return Err(ScopeError::io(&config_path, error)),
    };
    let config: ArchConfig = toml::from_str(&source)
        .map_err(|error| ScopeError::InvalidInput(format!("invalid arch config: {error}")))?;
    validate_arch_config(&config)?;
    Ok(config)
}

pub fn arch_check(store: &Store, config: &ArchConfig) -> ScopeResult<ArchCheckResult> {
    let edges = store.query_file_edges()?;
    let (checked_layered_edges, violations) = arch_check_edges(config, &edges)?;

    Ok(ArchCheckResult {
        config_path: RepoPath::from(".scope/arch.toml"),
        checked_edges: edges.len(),
        checked_layered_edges,
        violations,
    })
}

pub fn arch_init(repo_root: &Path) -> ScopeResult<ArchInitResult> {
    let layers = detect_arch_init_layers(repo_root)?;
    if layers.is_empty() {
        return Err(ScopeError::InvalidInput(
            "scope arch init could not detect any common layer directories".to_string(),
        ));
    }

    let rules = build_arch_init_rules(&layers);
    let config = ArchConfig {
        layers: layers.clone(),
        rules: rules.clone(),
        ..ArchConfig::default()
    };
    validate_arch_config(&config)?;

    let config_path = repo_root.join(".scope/arch.toml");
    if config_path.exists() {
        return Err(ScopeError::InvalidInput(format!(
            "arch config already exists at {}",
            config_path.display()
        )));
    }

    let source = render_arch_init_toml(&layers, &rules)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| ScopeError::io(parent, error))?;
    }
    fs::write(&config_path, source).map_err(|error| ScopeError::io(&config_path, error))?;

    Ok(ArchInitResult {
        config_path: RepoPath::from(".scope/arch.toml"),
        layers,
        rules,
        message: ARCH_INIT_MESSAGE.to_string(),
    })
}

pub fn arch_explain(
    repo_root: &Path,
    _config_path: &Path,
    target: &str,
) -> ScopeResult<ArchExplainResult> {
    let config = load_arch_config(repo_root)?;
    let target_path = RepoPath::from(target);
    let layer = resolve_layer(&config.layers, &target_path)?.cloned();
    let db_path = repo_root.join(".scope").join("index.db");
    let store = Store::open(&db_path)?;
    let indexed_files = store.list_indexed_files()?;
    let mut warnings = Vec::new();

    if config.layers.is_empty() {
        warnings.push("arch config defines no layers".to_string());
    }
    if !indexed_files.iter().any(|path| path == &target_path) {
        warnings.push(format!("File '{}' is not indexed", target_path.0));
    }
    if layer.is_none() {
        warnings.push(format!(
            "File '{}' is not in any defined layer",
            target_path.0
        ));
    }

    let applicable_rules: Vec<ArchRule> = layer
        .as_ref()
        .map(|layer| {
            config
                .rules
                .iter()
                .filter(|rule| rule.from == layer.name)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let rules = applicable_rules
        .iter()
        .cloned()
        .map(|rule| ArchExplainRule {
            applies: true,
            reason: layer.as_ref().map(|layer| {
                format!(
                    "rule applies because '{}' matches layer '{}'",
                    target_path.0, layer.name
                )
            }),
            rule,
        })
        .collect();

    let mut imports = Vec::new();
    for dependency in store.query_deps(&target_path)? {
        let to_file = dependency.path;
        let to_layer = resolve_layer(&config.layers, &to_file)?.map(|layer| layer.name.clone());
        let rule = applicable_rules
            .iter()
            .find(|rule| {
                to_layer.as_ref().is_some_and(|to_layer| {
                    rule.may_not_import.iter().any(|target| target == to_layer)
                })
            })
            .cloned();
        imports.push(ArchExplainImport {
            to_file,
            to_layer,
            rule: rule.clone(),
            violation: rule.is_some(),
            rule_source: rule.as_ref().map(|rule| rule.from.clone()),
        });
    }
    imports.sort_by(|left, right| left.to_file.0.cmp(&right.to_file.0));

    Ok(ArchExplainResult {
        target: target_path,
        layer,
        rules,
        imports,
        warnings,
    })
}

pub fn arch_check_edges(
    config: &ArchConfig,
    edges: &[ArchFileEdge],
) -> ScopeResult<(usize, Vec<ArchViolation>)> {
    let mut violations = Vec::new();
    let mut checked_layered_edges = 0;

    for edge in edges {
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

    Ok((checked_layered_edges, violations))
}

fn detect_arch_init_layers(repo_root: &Path) -> ScopeResult<Vec<ArchLayer>> {
    let mut matched_paths = Vec::new();
    collect_arch_init_layer_dirs(repo_root, repo_root, 0, &mut matched_paths)?;
    matched_paths.sort();
    matched_paths.dedup();

    let mut layers = Vec::new();
    let mut seen_names = HashSet::new();
    for (name, pattern) in matched_paths {
        if !seen_names.insert(name.clone()) {
            continue;
        }
        layers.push(ArchLayer {
            name,
            pattern,
            description: None,
        });
    }
    Ok(layers)
}

fn collect_arch_init_layer_dirs(
    repo_root: &Path,
    current: &Path,
    depth: usize,
    matched_paths: &mut Vec<(String, String)>,
) -> ScopeResult<()> {
    if depth > ARCH_INIT_MAX_DEPTH {
        return Ok(());
    }

    let entries = fs::read_dir(current).map_err(|error| ScopeError::io(current, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| ScopeError::io(current, error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| ScopeError::io(&path, error))?;
        if !file_type.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if ARCH_INIT_SKIP_DIRS.iter().any(|skip| skip == &name) {
            continue;
        }

        if let Some(layer_name) = canonical_arch_layer_name(name) {
            let relative = path.strip_prefix(repo_root).map_err(|error| {
                ScopeError::Internal(format!(
                    "failed to derive relative layer path for {}: {error}",
                    path.display()
                ))
            })?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            matched_paths.push((layer_name.to_string(), format!("{relative}/**")));
        }

        collect_arch_init_layer_dirs(repo_root, &path, depth + 1, matched_paths)?;
    }

    Ok(())
}

fn canonical_arch_layer_name(name: &str) -> Option<&'static str> {
    ARCH_INIT_LAYER_ALIASES
        .iter()
        .find_map(|(canonical, aliases)| {
            aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
                .then_some(*canonical)
        })
}

fn build_arch_init_rules(layers: &[ArchLayer]) -> Vec<ArchRule> {
    let layer_names: HashSet<&str> = layers.iter().map(|layer| layer.name.as_str()).collect();
    let mut rules = Vec::new();

    if layer_names.contains("services") && layer_names.contains("routes") {
        rules.push(ArchRule {
            from: "services".to_string(),
            may_not_import: vec!["routes".to_string()],
            message: Some("services must not import routes".to_string()),
        });
    }

    let model_targets: Vec<String> = ["services", "routes"]
        .into_iter()
        .filter(|name| layer_names.contains(*name))
        .map(str::to_string)
        .collect();
    if layer_names.contains("models") && !model_targets.is_empty() {
        rules.push(ArchRule {
            from: "models".to_string(),
            may_not_import: model_targets,
            message: Some("models must not import services or routes".to_string()),
        });
    }

    let utils_targets: Vec<String> = ["models", "services", "routes"]
        .into_iter()
        .filter(|name| layer_names.contains(*name))
        .map(str::to_string)
        .collect();
    if layer_names.contains("utils") && !utils_targets.is_empty() {
        rules.push(ArchRule {
            from: "utils".to_string(),
            may_not_import: utils_targets,
            message: Some("utils must stay infrastructure-only".to_string()),
        });
    }

    rules
}

fn render_arch_init_toml(layers: &[ArchLayer], rules: &[ArchRule]) -> ScopeResult<String> {
    toml::to_string_pretty(&StarterArchConfig {
        layers: layers.to_vec(),
        rules: rules.to_vec(),
    })
    .map_err(|error| ScopeError::Serialization(error.to_string()))
}

fn validate_arch_config(config: &ArchConfig) -> ScopeResult<()> {
    validate_test_config(&config.tests)?;
    validate_entry_point_config(&config.entry_points)?;
    validate_capability_config(&config.capabilities)?;
    validate_gate_config(&config.gates)?;

    if config.layers.is_empty() {
        return Ok(());
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

pub fn validate_test_config(config: &TestConfig) -> ScopeResult<()> {
    for pattern in &config.patterns {
        Pattern::new(pattern).map_err(|error| {
            ScopeError::InvalidInput(format!("invalid test pattern '{pattern}': {error}"))
        })?;
    }
    for pattern in &config.exclude_patterns {
        Pattern::new(pattern).map_err(|error| {
            ScopeError::InvalidInput(format!("invalid test exclude pattern '{pattern}': {error}"))
        })?;
    }
    Ok(())
}

pub fn validate_entry_point_config(config: &[crate::EntryPointConfig]) -> ScopeResult<()> {
    for entry_point in config {
        Pattern::new(&entry_point.pattern).map_err(|error| {
            ScopeError::InvalidInput(format!(
                "invalid entry point pattern '{}': {error}",
                entry_point.pattern
            ))
        })?;
    }
    Ok(())
}

pub fn validate_capability_config(config: &[crate::CapabilityConfig]) -> ScopeResult<()> {
    let mut names = HashSet::new();
    for capability in config {
        if capability.name.trim().is_empty() {
            return Err(ScopeError::InvalidInput(
                "capability name may not be empty".to_string(),
            ));
        }
        if !names.insert(capability.name.as_str()) {
            return Err(ScopeError::InvalidInput(format!(
                "duplicate capability name: {}",
                capability.name
            )));
        }
        if capability.pattern.is_none() && capability.symbols.is_empty() {
            return Err(ScopeError::InvalidInput(format!(
                "capability '{}' must declare a pattern or symbols",
                capability.name
            )));
        }
        if let Some(pattern) = &capability.pattern {
            Pattern::new(pattern).map_err(|error| {
                ScopeError::InvalidInput(format!(
                    "invalid capability pattern '{}': {error}",
                    pattern
                ))
            })?;
        }
        for pattern in &capability.expected_callers {
            Pattern::new(pattern).map_err(|error| {
                ScopeError::InvalidInput(format!(
                    "invalid capability expected_callers pattern '{}': {error}",
                    pattern
                ))
            })?;
        }
    }
    Ok(())
}

pub fn validate_gate_config(config: &[GateConfig]) -> ScopeResult<()> {
    for gate in config {
        if gate.min.is_none()
            && gate.max.is_none()
            && gate.min_delta.is_none()
            && gate.max_delta.is_none()
            && !gate.skip
        {
            return Err(ScopeError::InvalidInput(format!(
                "gate '{:?}' must declare at least one threshold or set skip = true",
                gate.metric
            )));
        }

        if let (Some(min), Some(max)) = (gate.min, gate.max) {
            if min > max {
                return Err(ScopeError::InvalidInput(format!(
                    "gate '{:?}' min may not exceed max",
                    gate.metric
                )));
            }
        }

        if let (Some(min_delta), Some(max_delta)) = (gate.min_delta, gate.max_delta) {
            if min_delta > max_delta {
                return Err(ScopeError::InvalidInput(format!(
                    "gate '{:?}' min_delta may not exceed max_delta",
                    gate.metric
                )));
            }
        }

        match gate.metric {
            GateMetric::HealthScore
            | GateMetric::ImportsUnresolvedPct
            | GateMetric::ImportsResolvedPct => {
                for (label, value) in [("min", gate.min), ("max", gate.max)] {
                    if let Some(value) = value {
                        if !(0.0..=100.0).contains(&value) {
                            return Err(ScopeError::InvalidInput(format!(
                                "gate '{:?}' {label} must be between 0 and 100",
                                gate.metric
                            )));
                        }
                    }
                }
            }
            GateMetric::HealthScoreDelta => {
                if gate.min.is_some() || gate.max.is_some() {
                    return Err(ScopeError::InvalidInput(
                        "gate 'HealthScoreDelta' must use min_delta/max_delta rather than min/max"
                            .to_string(),
                    ));
                }
            }
            _ => {
                for (label, value) in [("min", gate.min), ("max", gate.max)] {
                    if let Some(value) = value {
                        if value < 0.0 {
                            return Err(ScopeError::InvalidInput(format!(
                                "gate '{:?}' {label} must be non-negative",
                                gate.metric
                            )));
                        }
                    }
                }
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
    use crate::CapabilityConfig;

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
            entry_points: Vec::new(),
            capabilities: Vec::new(),
            gates: Vec::new(),
            tests: TestConfig::default(),
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

    #[test]
    fn validates_invalid_test_pattern() {
        let error = validate_test_config(&TestConfig {
            patterns: vec!["[".to_string()],
            exclude_patterns: Vec::new(),
        })
        .unwrap_err();

        assert!(error.to_string().contains("invalid test pattern"));
    }

    #[test]
    fn validates_invalid_entry_point_pattern() {
        let error = validate_entry_point_config(&[crate::EntryPointConfig {
            pattern: "[".to_string(),
        }])
        .unwrap_err();

        assert!(error.to_string().contains("invalid entry point pattern"));
    }

    #[test]
    fn validates_capability_requirements() {
        let error = validate_capability_config(&[CapabilityConfig {
            name: "network".to_string(),
            pattern: None,
            symbols: Vec::new(),
            expected_callers: Vec::new(),
        }])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("must declare a pattern or symbols"));

        let error = validate_capability_config(&[CapabilityConfig {
            name: "network".to_string(),
            pattern: Some("[".to_string()),
            symbols: Vec::new(),
            expected_callers: Vec::new(),
        }])
        .unwrap_err();
        assert!(error.to_string().contains("invalid capability pattern"));

        let error = validate_capability_config(&[CapabilityConfig {
            name: "network".to_string(),
            pattern: Some("src/**".to_string()),
            symbols: Vec::new(),
            expected_callers: vec!["[".to_string()],
        }])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid capability expected_callers pattern"));
    }

    #[test]
    fn rejects_health_score_delta_min_and_max_thresholds() {
        let error = validate_gate_config(&[GateConfig {
            metric: GateMetric::HealthScoreDelta,
            min: Some(90.0),
            max: None,
            min_delta: None,
            max_delta: None,
            severity: crate::GateSeverity::Error,
            message: None,
            skip: false,
        }])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("must use min_delta/max_delta rather than min/max"));
    }

    #[test]
    fn allows_negative_health_score_delta_thresholds() {
        validate_gate_config(&[GateConfig {
            metric: GateMetric::HealthScoreDelta,
            min: None,
            max: None,
            min_delta: Some(-5.0),
            max_delta: Some(0.0),
            severity: crate::GateSeverity::Warning,
            message: None,
            skip: false,
        }])
        .unwrap();
    }

    #[test]
    fn skip_gates_may_omit_thresholds() {
        validate_gate_config(&[GateConfig {
            metric: GateMetric::Cycles,
            min: None,
            max: None,
            min_delta: None,
            max_delta: None,
            severity: crate::GateSeverity::Warning,
            message: None,
            skip: true,
        }])
        .unwrap();
    }
}
