use serde::Serialize;

use crate::{
    json::JsonEnvelope,
    model::{
        ArchCheckResult, BranchDiffResult, Certainty, ContextResult, CyclesResult,
        DependencyRecord, EdgeKind, ImpactChangeType, ImpactTraversalRule, NodeKind,
        PublicSurface, PublicSurfaceDiff, RenamePlan, RepoPath, RiskResult,
        SnapshotDeleteResult, SnapshotDiffResult, SnapshotListResult, SnapshotSaveResult, Span,
        StabilityResult, SymbolKind, SymbolRecord, TestMapBuildResult, TestMapCoveredByResult,
        TestMapCoversResult, TestMapUncoveredResult, TraversalRecord, TreeResult, UnusedResult,
        Visibility,
    },
    DatabaseInfo, IndexHealthStats,
};

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BenchmarkMutationSummary {
    pub target_file: RepoPath,
    pub change_kind: &'static str,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BenchmarkPhaseSummary {
    pub avg_ms: u128,
    pub min_ms: u128,
    pub max_ms: u128,
    pub files_processed_avg: usize,
    pub changed_files_avg: usize,
    pub deleted_files_avg: usize,
    pub affected_files_avg: usize,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BenchmarkComparisonSummary {
    pub saved_ms: i128,
    pub incremental_pct_of_full: u32,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BenchmarkSummary {
    pub indexed_files: usize,
    pub mutation: BenchmarkMutationSummary,
    pub full: BenchmarkPhaseSummary,
    pub incremental: BenchmarkPhaseSummary,
    pub comparison: BenchmarkComparisonSummary,
}

#[derive(Debug, Serialize)]
pub struct IndexData {
    pub repo_root: RepoPath,
    pub no_git: bool,
    pub watch: bool,
    pub database: DatabaseInfo,
    pub indexed_files: usize,
}

#[derive(Debug, Serialize)]
pub struct DepsData {
    pub target: RepoPath,
    pub reverse: bool,
    pub transitive: bool,
    pub depth: Option<usize>,
    pub dependencies: Vec<DependencyRecord>,
}

#[derive(Debug, Serialize)]
pub struct SymbolsData {
    pub target: RepoPath,
    pub public_only: bool,
    pub kind: Option<SymbolKind>,
    pub symbols: Vec<SymbolRecord>,
}

#[derive(Debug, Serialize)]
pub struct CallsData {
    pub symbol: String,
    pub transitive: bool,
    pub traversals: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct ImpactSummary {
    pub total: usize,
    pub exact: usize,
    pub resolved: usize,
    pub heuristic: usize,
    pub dynamic: usize,
}

#[derive(Debug, Serialize)]
pub struct ImpactGroups {
    pub exact: Vec<TraversalRecord>,
    pub resolved: Vec<TraversalRecord>,
    pub heuristic: Vec<TraversalRecord>,
    pub dynamic: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct ImpactData {
    pub target: String,
    pub change_type: String,
    pub depth: Option<usize>,
    pub impacted: Vec<TraversalRecord>,
    pub grouped: ImpactGroups,
    pub summary: ImpactSummary,
    pub risk: &'static str,
    pub traversal_rule: ImpactTraversalRule,
}

#[derive(Debug, Serialize)]
pub struct ExplainData {
    pub target: String,
    pub to: Option<String>,
    pub depth: Option<usize>,
    pub traversals: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct WhyData {
    pub from: String,
    pub to: String,
    pub depth: Option<usize>,
    pub path: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct ContextData {
    pub result: ContextResult,
}

#[derive(Debug, Serialize)]
pub struct DoctorData {
    pub fix: bool,
    pub schema_version: u32,
    pub stats: IndexHealthStats,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub status: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkData {
    pub fixture: Option<String>,
    pub iterations: Option<u32>,
    pub summary: BenchmarkSummary,
}

#[derive(Debug, Serialize)]
pub struct ArchCheckData {
    pub config_path: RepoPath,
    pub checked_edges: usize,
    pub checked_layered_edges: usize,
    pub violations: Vec<crate::ArchViolation>,
}

#[derive(Debug, Serialize)]
pub struct StabilityData {
    pub result: StabilityResult,
}

#[derive(Debug, Serialize)]
pub struct RiskData {
    pub result: RiskResult,
}

#[derive(Debug, Serialize)]
pub struct UnusedData {
    pub result: UnusedResult,
}

#[derive(Debug, Serialize)]
pub struct CyclesData {
    pub result: CyclesResult,
}

#[derive(Debug, Serialize)]
pub struct DiffData {
    pub result: BranchDiffResult,
}

#[derive(Debug, Serialize)]
pub struct TreeData {
    pub result: TreeResult,
}

#[derive(Debug, Serialize)]
pub struct SurfaceData {
    pub target: RepoPath,
    pub surface: PublicSurface,
}

#[derive(Debug, Serialize)]
pub struct SurfaceDiffData {
    pub before: RepoPath,
    pub after: RepoPath,
    pub diff: PublicSurfaceDiff,
}

#[derive(Debug, Serialize)]
pub struct RenamePlanData {
    pub result: RenamePlan,
}

#[derive(Debug, Serialize)]
pub struct SnapshotSaveData {
    pub result: SnapshotSaveResult,
}

#[derive(Debug, Serialize)]
pub struct SnapshotListData {
    pub result: SnapshotListResult,
}

#[derive(Debug, Serialize)]
pub struct SnapshotDeleteData {
    pub result: SnapshotDeleteResult,
}

#[derive(Debug, Serialize)]
pub struct SnapshotDiffData {
    pub result: SnapshotDiffResult,
}

#[derive(Debug, Serialize)]
pub struct TestMapBuildData {
    pub result: TestMapBuildResult,
}

#[derive(Debug, Serialize)]
pub struct TestMapCoversData {
    pub result: TestMapCoversResult,
}

#[derive(Debug, Serialize)]
pub struct TestMapCoveredByData {
    pub result: TestMapCoveredByResult,
}

#[derive(Debug, Serialize)]
pub struct TestMapUncoveredData {
    pub result: TestMapUncoveredResult,
}

pub fn index(
    repo_root: String,
    no_git: bool,
    watch: bool,
    database: DatabaseInfo,
    indexed_files: usize,
) -> JsonEnvelope<IndexData> {
    JsonEnvelope::success(
        "index",
        IndexData {
            repo_root: RepoPath::from(repo_root),
            no_git,
            watch,
            database,
            indexed_files,
        },
    )
}

pub fn deps(
    target: String,
    reverse: bool,
    transitive: bool,
    depth: Option<usize>,
    dependencies: Vec<DependencyRecord>,
) -> JsonEnvelope<DepsData> {
    JsonEnvelope::success(
        "deps",
        DepsData {
            target: RepoPath::from(target),
            reverse,
            transitive,
            depth,
            dependencies,
        },
    )
}

pub fn symbols(
    target: String,
    public_only: bool,
    kind: Option<SymbolKind>,
    symbols: Vec<SymbolRecord>,
) -> JsonEnvelope<SymbolsData> {
    JsonEnvelope::success(
        "symbols",
        SymbolsData {
            target: RepoPath::from(target),
            public_only,
            kind,
            symbols,
        },
    )
}

pub fn calls(
    symbol: String,
    transitive: bool,
    traversals: Vec<TraversalRecord>,
) -> JsonEnvelope<CallsData> {
    let envelope = CallsData {
        symbol,
        transitive,
        traversals,
    };

    if transitive {
        JsonEnvelope::stub("calls", envelope)
    } else {
        JsonEnvelope::success("calls", envelope)
    }
}

pub fn callers(
    symbol: String,
    transitive: bool,
    traversals: Vec<TraversalRecord>,
) -> JsonEnvelope<CallsData> {
    let envelope = CallsData {
        symbol,
        transitive,
        traversals,
    };

    if transitive {
        JsonEnvelope::stub("callers", envelope)
    } else {
        JsonEnvelope::success("callers", envelope)
    }
}

pub fn impact(
    target: String,
    change_type: String,
    depth: Option<usize>,
    impacted: Vec<TraversalRecord>,
) -> JsonEnvelope<ImpactData> {
    let traversal_rule = impact_rule(&change_type);
    let grouped = impact_groups(&impacted);
    let summary = impact_summary(&impacted);
    JsonEnvelope::success(
        "impact",
        ImpactData {
            target,
            change_type,
            depth,
            risk: impact_risk_label(&impacted),
            impacted,
            grouped,
            summary,
            traversal_rule,
        },
    )
}

fn impact_summary(impacted: &[TraversalRecord]) -> ImpactSummary {
    ImpactSummary {
        total: impacted.len(),
        exact: impacted
            .iter()
            .filter(|record| matches!(record.certainty, Certainty::Exact))
            .count(),
        resolved: impacted
            .iter()
            .filter(|record| matches!(record.certainty, Certainty::Resolved))
            .count(),
        heuristic: impacted
            .iter()
            .filter(|record| matches!(record.certainty, Certainty::Heuristic))
            .count(),
        dynamic: impacted
            .iter()
            .filter(|record| matches!(record.certainty, Certainty::Dynamic))
            .count(),
    }
}

fn impact_groups(impacted: &[TraversalRecord]) -> ImpactGroups {
    let by_certainty = |certainty| {
        impacted
            .iter()
            .filter(|record| record.certainty == certainty)
            .cloned()
            .collect()
    };

    ImpactGroups {
        exact: by_certainty(Certainty::Exact),
        resolved: by_certainty(Certainty::Resolved),
        heuristic: by_certainty(Certainty::Heuristic),
        dynamic: by_certainty(Certainty::Dynamic),
    }
}

fn impact_risk_label(impacted: &[TraversalRecord]) -> &'static str {
    if impacted.is_empty() {
        "low"
    } else if impacted
        .iter()
        .any(|record| matches!(record.certainty, Certainty::Dynamic))
    {
        "high"
    } else if impacted
        .iter()
        .any(|record| matches!(record.certainty, Certainty::Heuristic))
    {
        "medium"
    } else {
        "low"
    }
}

pub fn impact_rule(change_type: &str) -> ImpactTraversalRule {
    match change_type {
        "body" => ImpactTraversalRule {
            change_type: ImpactChangeType::Body,
            summary: "Implementation changed without changing the public shape".to_string(),
            traversal_strategy: "Reverse call graph only".to_string(),
            primary_blast_radius: "Direct callers and behavioral tests".to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call],
            include_transitive: false,
            default_max_distance: Some(1),
            include_re_exports: false,
            include_importers: false,
            include_callers: true,
            include_visibility_boundary: false,
        },
        "signature" => ImpactTraversalRule {
            change_type: ImpactChangeType::Signature,
            summary: "Parameters, return type, or callable contract changed".to_string(),
            traversal_strategy: "Reverse call graph plus importer/re-export expansion".to_string(),
            primary_blast_radius: "All callers, transitive wrappers, and exported API consumers"
                .to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
            include_transitive: true,
            default_max_distance: None,
            include_re_exports: true,
            include_importers: true,
            include_callers: true,
            include_visibility_boundary: false,
        },
        "rename" => ImpactTraversalRule {
            change_type: ImpactChangeType::Rename,
            summary: "A symbol, file, or module name changed".to_string(),
            traversal_strategy: "Traverse all reference-bearing edges".to_string(),
            primary_blast_radius: "References, import sites, and re-export chains".to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
            include_transitive: true,
            default_max_distance: None,
            include_re_exports: true,
            include_importers: true,
            include_callers: true,
            include_visibility_boundary: false,
        },
        "delete" => ImpactTraversalRule {
            change_type: ImpactChangeType::Delete,
            summary: "The target was removed entirely".to_string(),
            traversal_strategy: "Traverse all reverse dependencies and reference edges".to_string(),
            primary_blast_radius: "All callers and importers that would break".to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
            include_transitive: true,
            default_max_distance: None,
            include_re_exports: true,
            include_importers: true,
            include_callers: true,
            include_visibility_boundary: false,
        },
        "visibility" => ImpactTraversalRule {
            change_type: ImpactChangeType::Visibility,
            summary: "Accessibility changed across a visibility boundary".to_string(),
            traversal_strategy: "Inspect external import/call edges across the narrowed boundary"
                .to_string(),
            primary_blast_radius: "External consumers and re-export paths outside the boundary"
                .to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
            include_transitive: true,
            default_max_distance: None,
            include_re_exports: true,
            include_importers: true,
            include_callers: true,
            include_visibility_boundary: true,
        },
        "side-effect" => ImpactTraversalRule {
            change_type: ImpactChangeType::SideEffect,
            summary: "File-level initialization or execution behavior changed".to_string(),
            traversal_strategy: "Reverse file-import graph only".to_string(),
            primary_blast_radius: "Importers and transitive importers affected by init order"
                .to_string(),
            allowed_edge_kinds: vec![EdgeKind::Import],
            include_transitive: true,
            default_max_distance: None,
            include_re_exports: false,
            include_importers: true,
            include_callers: false,
            include_visibility_boundary: false,
        },
        _ => ImpactTraversalRule {
            change_type: ImpactChangeType::Body,
            summary: "Unknown change type; defaulting to the narrowest documented traversal"
                .to_string(),
            traversal_strategy: "Reverse call graph only".to_string(),
            primary_blast_radius: "Direct callers only".to_string(),
            allowed_edge_kinds: vec![EdgeKind::Call],
            include_transitive: false,
            default_max_distance: Some(1),
            include_re_exports: false,
            include_importers: false,
            include_callers: true,
            include_visibility_boundary: false,
        },
    }
}

pub fn explain(
    target: String,
    to: Option<String>,
    depth: Option<usize>,
    traversals: Vec<TraversalRecord>,
) -> JsonEnvelope<ExplainData> {
    JsonEnvelope::success(
        "explain",
        ExplainData {
            target,
            to,
            depth,
            traversals,
        },
    )
}

pub fn why(
    from: String,
    to: String,
    depth: Option<usize>,
    path: Vec<TraversalRecord>,
) -> JsonEnvelope<WhyData> {
    JsonEnvelope::success(
        "why",
        WhyData {
            from,
            to,
            depth,
            path,
        },
    )
}

pub fn context(result: ContextResult) -> JsonEnvelope<ContextData> {
    JsonEnvelope::success("context", ContextData { result })
}

pub fn doctor(fix: bool, stats: IndexHealthStats) -> JsonEnvelope<DoctorData> {
    let unresolved_status = if stats.unresolved_imports == 0 {
        "ok"
    } else {
        "warn"
    };
    let parse_status = if stats.parse_status.error == 0 {
        if stats.parse_status.partial == 0 {
            "ok"
        } else {
            "warn"
        }
    } else {
        "fail"
    };

    JsonEnvelope::success(
        "doctor",
        DoctorData {
            fix,
            schema_version: crate::INDEX_SCHEMA_VERSION,
            checks: vec![
                DoctorCheck {
                    name: "files_indexed",
                    status: if stats.files == 0 { "fail" } else { "ok" },
                    detail: format!("{} indexed file(s)", stats.files),
                },
                DoctorCheck {
                    name: "unresolved_imports",
                    status: unresolved_status,
                    detail: format!(
                        "{} unresolved non-external import(s)",
                        stats.unresolved_imports
                    ),
                },
                DoctorCheck {
                    name: "parse_status",
                    status: parse_status,
                    detail: format!(
                        "ok={}, partial={}, error={}",
                        stats.parse_status.ok, stats.parse_status.partial, stats.parse_status.error
                    ),
                },
            ],
            stats,
        },
    )
}

pub fn benchmark(
    fixture: Option<String>,
    iterations: Option<u32>,
    summary: BenchmarkSummary,
) -> JsonEnvelope<BenchmarkData> {
    let iterations = Some(iterations.unwrap_or(1));
    JsonEnvelope::success(
        "benchmark",
        BenchmarkData {
            fixture,
            iterations,
            summary,
        },
    )
}

pub fn arch_check(result: ArchCheckResult) -> JsonEnvelope<ArchCheckData> {
    JsonEnvelope::success(
        "arch-check",
        ArchCheckData {
            config_path: result.config_path,
            checked_edges: result.checked_edges,
            checked_layered_edges: result.checked_layered_edges,
            violations: result.violations,
        },
    )
}

pub fn stability(result: StabilityResult) -> JsonEnvelope<StabilityData> {
    JsonEnvelope::success("stability", StabilityData { result })
}

pub fn risk(result: RiskResult) -> JsonEnvelope<RiskData> {
    JsonEnvelope::success("risk", RiskData { result })
}

pub fn unused(result: UnusedResult) -> JsonEnvelope<UnusedData> {
    JsonEnvelope::success("unused", UnusedData { result })
}

pub fn cycles(result: CyclesResult) -> JsonEnvelope<CyclesData> {
    JsonEnvelope::success("cycles", CyclesData { result })
}

pub fn diff(result: BranchDiffResult) -> JsonEnvelope<DiffData> {
    JsonEnvelope::success("diff", DiffData { result })
}

pub fn tree(result: TreeResult) -> JsonEnvelope<TreeData> {
    JsonEnvelope::success("tree", TreeData { result })
}

pub fn rename_plan(result: RenamePlan) -> JsonEnvelope<RenamePlanData> {
    JsonEnvelope::success("rename-plan", RenamePlanData { result })
}

pub fn snapshot_save(result: SnapshotSaveResult) -> JsonEnvelope<SnapshotSaveData> {
    JsonEnvelope::success("snapshot-save", SnapshotSaveData { result })
}

pub fn snapshot_list(result: SnapshotListResult) -> JsonEnvelope<SnapshotListData> {
    JsonEnvelope::success("snapshot-list", SnapshotListData { result })
}

pub fn snapshot_delete(result: SnapshotDeleteResult) -> JsonEnvelope<SnapshotDeleteData> {
    JsonEnvelope::success("snapshot-delete", SnapshotDeleteData { result })
}

pub fn diff_snapshot(result: SnapshotDiffResult) -> JsonEnvelope<SnapshotDiffData> {
    JsonEnvelope::success("diff-snapshot", SnapshotDiffData { result })
}

pub fn test_map_build(result: TestMapBuildResult) -> JsonEnvelope<TestMapBuildData> {
    JsonEnvelope::success("test-map-build", TestMapBuildData { result })
}

pub fn test_map_covers(result: TestMapCoversResult) -> JsonEnvelope<TestMapCoversData> {
    JsonEnvelope::success("test-map-covers", TestMapCoversData { result })
}

pub fn test_map_covered_by(
    result: TestMapCoveredByResult,
) -> JsonEnvelope<TestMapCoveredByData> {
    JsonEnvelope::success("test-map-covered-by", TestMapCoveredByData { result })
}

pub fn test_map_uncovered(result: TestMapUncoveredResult) -> JsonEnvelope<TestMapUncoveredData> {
    JsonEnvelope::success("test-map-uncovered", TestMapUncoveredData { result })
}

pub fn surface(target: RepoPath, surface: PublicSurface) -> JsonEnvelope<SurfaceData> {
    JsonEnvelope::success("surface", SurfaceData { target, surface })
}

pub fn surface_diff(
    before: RepoPath,
    after: RepoPath,
    diff: PublicSurfaceDiff,
) -> JsonEnvelope<SurfaceDiffData> {
    JsonEnvelope::success(
        "surface-diff",
        SurfaceDiffData {
            before,
            after,
            diff,
        },
    )
}

pub fn mcp_stub_message() -> JsonEnvelope<TraversalRecord> {
    JsonEnvelope::stub(
        "scope-mcp",
        TraversalRecord {
            kind: NodeKind::Symbol,
            path: None,
            qualname: Some("scope-mcp".to_string()),
            edge_kind: EdgeKind::Dynamic,
            certainty: Certainty::Dynamic,
            reason: "scope-mcp is scaffolded but not implemented yet".to_string(),
            distance: 0,
        },
    )
}

pub fn placeholder_symbol(path: RepoPath, name: &str, kind: SymbolKind) -> SymbolRecord {
    SymbolRecord {
        file: path,
        name: name.to_string(),
        qualname: name.to_string(),
        kind,
        visibility: Visibility::Unknown,
        exported: false,
        span: Span {
            start_byte: 0,
            end_byte: 0,
            start_line: 0,
            end_line: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_rules_cover_all_supported_change_types() {
        let cases = [
            ("body", vec![EdgeKind::Call], false, Some(1), false, true),
            (
                "signature",
                vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
                true,
                None,
                true,
                true,
            ),
            (
                "rename",
                vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
                true,
                None,
                true,
                true,
            ),
            (
                "delete",
                vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
                true,
                None,
                true,
                true,
            ),
            (
                "visibility",
                vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export],
                true,
                None,
                true,
                true,
            ),
            (
                "side-effect",
                vec![EdgeKind::Import],
                true,
                None,
                true,
                false,
            ),
        ];

        for (change_type, expected_edges, transitive, max_distance, importers, callers) in cases {
            let rule = impact_rule(change_type);
            assert_eq!(
                rule.allowed_edge_kinds, expected_edges,
                "unexpected edge kinds for {change_type}"
            );
            assert_eq!(
                rule.include_transitive, transitive,
                "unexpected transitive flag for {change_type}"
            );
            assert_eq!(
                rule.default_max_distance, max_distance,
                "unexpected max distance for {change_type}"
            );
            assert_eq!(
                rule.include_importers, importers,
                "unexpected importer flag for {change_type}"
            );
            assert_eq!(
                rule.include_callers, callers,
                "unexpected caller flag for {change_type}"
            );
        }
    }

    #[test]
    fn impact_stub_embeds_traversal_rule_metadata() {
        let traversals = vec![TraversalRecord {
            kind: NodeKind::Symbol,
            path: Some(RepoPath::from("src/parser.rs")),
            qualname: Some("parser::parse".to_string()),
            edge_kind: EdgeKind::Call,
            certainty: Certainty::Resolved,
            reason: "calls parser::parse directly".to_string(),
            distance: 1,
        }];
        let envelope = impact(
            "parser::parse".to_string(),
            "signature".to_string(),
            Some(3),
            traversals,
        );
        assert!(matches!(envelope.status, crate::json::JsonStatus::Ok));
        assert_eq!(envelope.data.change_type, "signature");
        assert_eq!(envelope.data.depth, Some(3));
        assert_eq!(envelope.data.risk, "low");
        assert_eq!(envelope.data.summary.total, 1);
        assert_eq!(envelope.data.summary.resolved, 1);
        assert_eq!(envelope.data.summary.exact, 0);
        assert_eq!(envelope.data.summary.heuristic, 0);
        assert_eq!(envelope.data.summary.dynamic, 0);
        assert_eq!(envelope.data.grouped.resolved.len(), 1);
        assert_eq!(envelope.data.grouped.exact.len(), 0);
        assert_eq!(envelope.data.grouped.heuristic.len(), 0);
        assert_eq!(envelope.data.grouped.dynamic.len(), 0);
        assert_eq!(
            envelope.data.traversal_rule.change_type,
            ImpactChangeType::Signature
        );
        assert!(envelope.data.traversal_rule.include_re_exports);
        assert!(envelope.data.traversal_rule.include_importers);
        assert!(envelope.data.traversal_rule.include_callers);
        assert_eq!(
            envelope.data.traversal_rule.allowed_edge_kinds,
            vec![EdgeKind::Call, EdgeKind::Import, EdgeKind::Export]
        );
    }
}
