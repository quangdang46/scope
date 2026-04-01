use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{rejection::QueryRejection, Query, RawQuery, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use tower::ServiceExt;

use crate::{
    execute_query, load_arch_config,
    model::{CochangeSort, CycleSeverity, ImpactChangeType, RiskSort, StabilitySort, SymbolKind},
    query_runtime::{
        CallsQuery, ContextQuery, DepsQuery, ExplainQuery, ImpactQuery, QueryEngine, QueryRequest,
        SymbolsQuery, WhyQuery,
    },
    stub, DatabaseInfo, IndexHealthStats, QuerySession, RepoPath, RuntimePaths, ScopeError,
    ScopeResult, Store,
};

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub port: u16,
    pub open: bool,
    pub no_ui: bool,
}

#[derive(Debug, Clone)]
pub struct ServeState {
    pub paths: RuntimePaths,
}

#[derive(Debug, Serialize)]
pub struct ServeStatusData {
    pub repo_root: String,
    pub database: DatabaseInfo,
    pub stats: IndexHealthStats,
}

const WEB_UI_HTML: &str = include_str!("../web_ui/dist/index.html");

pub fn build_router(state: Arc<ServeState>, no_ui: bool) -> Router {
    let mut app = Router::new()
        .route("/api/status", get(api_status))
        .route("/api/deps", get(api_deps))
        .route("/api/symbols", get(api_symbols))
        .route("/api/calls", get(api_calls))
        .route("/api/callers", get(api_callers))
        .route("/api/impact", get(api_impact))
        .route("/api/explain", get(api_explain))
        .route("/api/why", get(api_why))
        .route("/api/context", get(api_context))
        .route("/api/report", get(api_report))
        .route("/api/gate", get(api_gate))
        .route("/api/query", get(api_query))
        .route("/api/unused", get(api_unused))
        .route("/api/risk", get(api_risk))
        .route("/api/stability", get(api_stability))
        .route("/api/cochange", get(api_cochange))
        .route("/api/audit", get(api_audit))
        .route("/api/surface", get(api_surface))
        .route("/api/surface/diff", get(api_surface_diff))
        .route("/api/cycles", get(api_cycles))
        .route("/api/tree", get(api_tree))
        .route("/api/simulate/extract", get(api_simulate_extract))
        .route("/api/entry/list", get(api_entry_list))
        .route("/api/entry/cone", get(api_entry_cone))
        .route("/api/entry/reaches", get(api_entry_reaches))
        .route("/api/entry/unreachable", get(api_entry_unreachable))
        .route("/api/snapshot/list", get(api_snapshot_list))
        .with_state(state);

    if no_ui {
        app = app.fallback(not_found_json);
    } else {
        app = app.fallback(serve_ui);
    }

    app
}

pub async fn run_server(paths: RuntimePaths, options: ServeOptions) -> ScopeResult<()> {
    let state = Arc::new(ServeState { paths });
    let app = build_router(state, options.no_ui);
    let requested_address = SocketAddr::from(([127, 0, 0, 1], options.port));
    let listener = tokio::net::TcpListener::bind(requested_address)
        .await
        .map_err(|error| ScopeError::io(format!("bind {requested_address}"), error))?;
    let local_address = listener
        .local_addr()
        .map_err(|error| ScopeError::io("listener local address", error))?;

    if options.open {
        let _ = open::that(format!("http://{local_address}"));
    }

    eprintln!("scope serve: http://{local_address}");
    axum::serve(listener, app)
        .await
        .map_err(|error| ScopeError::Internal(error.to_string()))
}

async fn serve_ui() -> Html<&'static str> {
    Html(WEB_UI_HTML)
}

async fn not_found_json() -> Response {
    json_error(
        StatusCode::NOT_FOUND,
        "serve",
        ScopeError::NotFound {
            kind: "route",
            value: "/".to_string(),
        },
    )
}

fn open_store(state: &ServeState) -> ScopeResult<Store> {
    Store::open(&state.paths.db_path)
}

fn status_payload(state: &ServeState) -> ScopeResult<crate::JsonEnvelope<ServeStatusData>> {
    let store = open_store(state)?;
    let schema_version = store.schema_version()?;
    let stats = store.index_health_stats()?;
    Ok(crate::JsonEnvelope::success(
        "serve-status",
        ServeStatusData {
            repo_root: state.paths.repo_root.display().to_string(),
            database: DatabaseInfo {
                path: state.paths.db_path.display().to_string(),
                schema_version,
            },
            stats,
        },
    ))
}

fn json_success<T: Serialize>(value: T) -> Response {
    Json(value).into_response()
}

fn json_error(status: StatusCode, command: &'static str, error: ScopeError) -> Response {
    let body = crate::JsonEnvelope::error(command, &error);
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn query_error(command: &'static str, error: ScopeError) -> Response {
    let status = match error {
        ScopeError::InvalidInput(_) | ScopeError::NotFound { .. } | ScopeError::IndexNotFound => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    json_error(status, command, error)
}

fn query_rejection_error(command: &'static str, rejection: QueryRejection) -> Response {
    query_error(command, ScopeError::InvalidInput(rejection.body_text()))
}

#[derive(Debug, Deserialize)]
struct DepsParams {
    file: String,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    transitive: bool,
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SymbolsParams {
    file: String,
    #[serde(default)]
    public_only: bool,
    kind: Option<SymbolKind>,
}

#[derive(Debug, Deserialize)]
struct CallsParams {
    symbol: String,
    #[serde(default)]
    transitive: bool,
}

#[derive(Debug, Deserialize)]
struct ImpactParams {
    target: String,
    change_type: ImpactChangeType,
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ExplainParams {
    target: String,
    to: Option<String>,
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WhyParams {
    from: String,
    to: String,
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ContextParams {
    target: String,
    change_type: ImpactChangeType,
    budget: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReportParams {
    compare: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GateParams {
    compare: Option<String>,
    #[serde(default)]
    strict: bool,
}

#[derive(Debug, Default)]
struct QueryParams {
    expr: Option<String>,
    exprs: Option<Vec<String>>,
}

impl QueryParams {
    fn from_query_pairs(pairs: Vec<(String, String)>) -> Result<Self, ScopeError> {
        let mut params = Self::default();
        for (key, value) in pairs {
            match key.as_str() {
                "expr" => params.expr = Some(value),
                "exprs" => params.exprs.get_or_insert_with(Vec::new).push(value),
                _ => {}
            }
        }
        Ok(params)
    }

    fn query_exprs(self) -> Result<Vec<String>, ScopeError> {
        match (self.expr, self.exprs) {
            (Some(_), Some(_)) => Err(ScopeError::InvalidInput(
                "serve query accepts either `expr` or `exprs`, but not both".to_string(),
            )),
            (Some(expr), None) => Ok(vec![expr]),
            (None, Some(exprs)) if exprs.is_empty() => Err(ScopeError::InvalidInput(
                "serve query parameter `exprs` must contain at least one expression".to_string(),
            )),
            (None, Some(exprs)) => Ok(exprs),
            (None, None) => Err(ScopeError::InvalidInput(
                "serve query requires `expr` or `exprs`".to_string(),
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RiskParams {
    file: Option<String>,
    #[serde(default = "default_days")]
    days: u32,
    threshold: Option<f64>,
    top: Option<usize>,
    #[serde(default)]
    sort: Option<RiskSort>,
}

#[derive(Debug, Deserialize)]
struct StabilityParams {
    file: Option<String>,
    flag_threshold: Option<f64>,
    #[serde(default)]
    sort: Option<StabilitySort>,
}

#[derive(Debug, Deserialize)]
struct CochangeParams {
    target: String,
    #[serde(default = "default_days")]
    days: u32,
    #[serde(default = "default_min_shared_commits")]
    min_shared_commits: usize,
    top: Option<usize>,
    #[serde(default)]
    sort: Option<CochangeSort>,
}

#[derive(Debug, Deserialize)]
struct EntryUnreachableParams {
    min_age_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AuditParams {
    capability: String,
}

#[derive(Debug, Deserialize)]
struct SurfaceParams {
    target: String,
}

#[derive(Debug, Deserialize)]
struct SurfaceDiffParams {
    before: String,
    after: String,
}

#[derive(Debug, Deserialize)]
struct CyclesParams {
    #[serde(default)]
    severity: Option<CycleSeverity>,
}

#[derive(Debug, Deserialize)]
struct TreeParams {
    target: String,
    #[serde(default)]
    reverse: bool,
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SimulateExtractParams {
    symbols: String,
    #[serde(rename = "into")]
    into_file: String,
}

#[derive(Debug, Deserialize)]
struct EntryTargetParams {
    target: String,
}

#[derive(Debug, Deserialize)]
struct EntryReachesParams {
    target: String,
}

fn default_days() -> u32 {
    90
}

fn default_min_shared_commits() -> usize {
    1
}

async fn api_status(State(state): State<Arc<ServeState>>) -> Response {
    match status_payload(&state) {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("serve-status", error),
    }
}

async fn api_deps(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<DepsParams>,
) -> Response {
    let request = QueryRequest::Deps(DepsQuery {
        file: params.file,
        reverse: params.reverse,
        transitive: params.transitive,
        depth: params.depth,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_symbols(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<SymbolsParams>,
) -> Response {
    let request = QueryRequest::Symbols(SymbolsQuery {
        file: params.file,
        public_only: params.public_only,
        kind: params.kind,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_calls(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<CallsParams>,
) -> Response {
    let request = QueryRequest::Calls(CallsQuery {
        symbol: params.symbol,
        transitive: params.transitive,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_callers(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<CallsParams>,
) -> Response {
    let request = QueryRequest::Callers(CallsQuery {
        symbol: params.symbol,
        transitive: params.transitive,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_impact(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<ImpactParams>,
) -> Response {
    let request = QueryRequest::Impact(ImpactQuery {
        target: params.target,
        change_type: impact_change_type_name(&params.change_type),
        depth: params.depth,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_explain(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<ExplainParams>,
) -> Response {
    let request = QueryRequest::Explain(ExplainQuery {
        target: params.target,
        to: params.to,
        depth: params.depth,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_why(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<WhyParams>,
) -> Response {
    let request = QueryRequest::Why(WhyQuery {
        from: params.from,
        to: params.to,
        depth: params.depth,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_context(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<ContextParams>,
) -> Response {
    let request = QueryRequest::Context(ContextQuery {
        targets: vec![params.target],
        change_type: impact_change_type_name(&params.change_type),
        budget: params.budget,
    });
    let command = request.command();
    match (|| {
        let store = open_store(&state)?;
        QueryEngine::new(&store).execute(request)
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error(command, error),
    }
}

async fn api_report(
    State(state): State<Arc<ServeState>>,
    params: Result<Query<ReportParams>, QueryRejection>,
) -> Response {
    let Query(params) = match params {
        Ok(params) => params,
        Err(rejection) => return query_rejection_error("report", rejection),
    };

    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_report(&config, params.compare.as_deref())?;
        Ok(stub::report(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("report", error),
    }
}

async fn api_gate(
    State(state): State<Arc<ServeState>>,
    params: Result<Query<GateParams>, QueryRejection>,
) -> Response {
    let Query(params) = match params {
        Ok(params) => params,
        Err(rejection) => return query_rejection_error("gate", rejection),
    };

    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_gate(&config, params.compare.as_deref(), params.strict)?;
        Ok(stub::gate(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("gate", error),
    }
}

async fn api_query(
    State(state): State<Arc<ServeState>>,
    params: Result<Query<Vec<(String, String)>>, QueryRejection>,
    raw_query: RawQuery,
) -> Response {
    let params = match params {
        Ok(Query(pairs)) => match QueryParams::from_query_pairs(pairs) {
            Ok(params) => params,
            Err(error) => return query_error("query", error),
        },
        Err(rejection) => {
            if raw_query.0.is_none() {
                QueryParams::default()
            } else {
                return query_rejection_error("query", rejection);
            }
        }
    };

    match (|| {
        let store = open_store(&state)?;
        let mut session = QuerySession::default();
        let exprs = params.query_exprs()?;
        let mut last_result = None;
        for expr in exprs {
            let result = execute_query(&expr, &store, &mut session)?;
            last_result = Some(stub::query(expr, result));
        }
        last_result.ok_or_else(|| {
            ScopeError::InvalidInput("serve query requires `expr` or `exprs`".to_string())
        })
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("query", error),
    }
}

async fn api_unused(State(state): State<Arc<ServeState>>) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let result = store.query_unused()?;
        Ok(stub::unused(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("unused", error),
    }
}

async fn api_risk(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<RiskParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let _ = refresh_git_churn(&state.paths.repo_root, &store, params.days);
        let result = store.query_risk(
            optional_repo_path(&params.file).as_ref(),
            params.days,
            params.threshold,
            params.top,
            params.sort.unwrap_or(RiskSort::Score),
        )?;
        Ok(stub::risk(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("risk", error),
    }
}

async fn api_stability(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<StabilityParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let result = store.query_stability(
            optional_repo_path(&params.file).as_ref(),
            params.flag_threshold,
            params.sort.unwrap_or(StabilitySort::Instability),
        )?;
        Ok(stub::stability(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("stability", error),
    }
}

async fn api_cochange(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<CochangeParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let _ = refresh_git_churn(&state.paths.repo_root, &store, params.days);
        let result = store.query_cochange(
            &RepoPath::from(params.target.clone()),
            params.days,
            params.min_shared_commits,
            params.top,
            params.sort.unwrap_or(CochangeSort::Score),
        )?;
        Ok(stub::cochange(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("cochange", error),
    }
}

async fn api_audit(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<AuditParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_audit(&config, &params.capability)?;
        Ok(stub::audit(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("audit", error),
    }
}

async fn api_surface(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<SurfaceParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let path = store.resolve_surface_target(&params.target)?;
        let surface = store.query_public_surface(&path)?;
        Ok(stub::surface(path, surface))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("surface", error),
    }
}

async fn api_surface_diff(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<SurfaceDiffParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let before = store.resolve_surface_target(&params.before)?;
        let after = store.resolve_surface_target(&params.after)?;
        let diff = store.diff_public_surface(&before, &after)?;
        Ok(stub::surface_diff(before, after, diff))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("surface-diff", error),
    }
}

async fn api_cycles(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<CyclesParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let result = store.query_cycles(params.severity)?;
        Ok(stub::cycles(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("cycles", error),
    }
}

async fn api_tree(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<TreeParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let result =
            store.query_tree(&RepoPath::from(params.target), params.reverse, params.depth)?;
        Ok(stub::tree(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("tree", error),
    }
}

async fn api_simulate_extract(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<SimulateExtractParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let symbols = params
            .symbols
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            return Err(ScopeError::InvalidInput(
                "simulate extract requires at least one symbol".to_string(),
            ));
        }
        let result =
            store.simulate_extract(&symbols, &RepoPath::from(params.into_file), &config)?;
        Ok(stub::simulate_extract(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("simulate-extract", error),
    }
}

async fn api_entry_list(State(state): State<Arc<ServeState>>) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_entry_list(&config)?;
        Ok(stub::entry_list(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("entry-list", error),
    }
}

async fn api_entry_cone(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<EntryTargetParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_entry_cone(&config, &RepoPath::from(params.target))?;
        Ok(stub::entry_cone(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("entry-cone", error),
    }
}

async fn api_entry_reaches(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<EntryReachesParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_entry_reaches(&config, &RepoPath::from(params.target))?;
        Ok(stub::entry_reaches(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("entry-reaches", error),
    }
}

async fn api_entry_unreachable(
    State(state): State<Arc<ServeState>>,
    Query(params): Query<EntryUnreachableParams>,
) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let config = load_arch_config(&state.paths.repo_root)?;
        let result = store.query_entry_unreachable(&config, params.min_age_days)?;
        Ok(stub::entry_unreachable(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("entry-unreachable", error),
    }
}

async fn api_snapshot_list(State(state): State<Arc<ServeState>>) -> Response {
    match (|| {
        let store = open_store(&state)?;
        let result = store.list_snapshots()?;
        Ok(stub::snapshot_list(result))
    })() {
        Ok(envelope) => json_success(envelope),
        Err(error) => query_error("snapshot-list", error),
    }
}

fn impact_change_type_name(change_type: &ImpactChangeType) -> String {
    match change_type {
        ImpactChangeType::Body => "body",
        ImpactChangeType::Signature => "signature",
        ImpactChangeType::Rename => "rename",
        ImpactChangeType::Delete => "delete",
        ImpactChangeType::Visibility => "visibility",
        ImpactChangeType::SideEffect => "side-effect",
    }
    .to_string()
}

fn refresh_git_churn(repo_root: &std::path::Path, store: &Store, days: u32) -> ScopeResult<()> {
    store.clear_file_churn()?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("log")
        .arg(format!("--since={} days ago", days))
        .arg("--format=%H|%ae|%ct")
        .arg("--name-only")
        .output()
        .map_err(|error| ScopeError::io("git log", error))?;

    if !output.status.success() {
        return Ok(());
    }

    let mut current_commit: Option<(String, String, i64)> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(header) = parse_git_log_header(trimmed) {
            current_commit = Some(header);
            continue;
        }
        let Some((sha, email, timestamp)) = current_commit.as_ref() else {
            continue;
        };
        let _ = store.persist_file_churn(
            &RepoPath::from(trimmed.to_string()),
            sha,
            Some(email.as_str()),
            Some(*timestamp),
        );
    }

    Ok(())
}

fn parse_git_log_header(line: &str) -> Option<(String, String, i64)> {
    let mut parts = line.split('|');
    let sha = parts.next()?.to_string();
    let email = parts.next()?.to_string();
    let timestamp = parts.next()?.parse().ok()?;
    if sha.is_empty() {
        return None;
    }
    Some((sha, email, timestamp))
}

fn optional_repo_path(value: &Option<String>) -> Option<RepoPath> {
    value.as_ref().map(|path| RepoPath::from(path.clone()))
}

#[cfg(test)]
mod tests {
    mod support {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support.rs"));
    }

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{adapter_for_language, scan_repo, ScanConfig, Store};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let salt = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("scope-serve-{prefix}-{nanos}-{salt}"))
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn fixture_root(name: &str) -> std::path::PathBuf {
        workspace_root().join("fixtures").join(name)
    }

    fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                copy_dir_recursive(&src_path, &dst_path);
            } else {
                if src_path
                    .strip_prefix(src)
                    .ok()
                    .and_then(|relative| relative.to_str())
                    == Some(".scope/index.db")
                {
                    continue;
                }
                fs::copy(&src_path, &dst_path).unwrap();
            }
        }
    }

    fn prepare_fixture_copy(name: &str) -> std::path::PathBuf {
        let src = fixture_root(name);
        let dst = unique_temp_dir(name);
        copy_dir_recursive(&src, &dst);
        dst
    }

    fn index_fixture(repo_root: &std::path::Path) {
        let store = Store::open(&repo_root.join(".scope/index.db")).unwrap();
        let entries = scan_repo(repo_root, &ScanConfig::default()).unwrap();
        let extracts: Vec<_> = entries
            .into_iter()
            .filter_map(|entry| {
                let adapter = adapter_for_language(entry.language)?;
                if !crate::adapters::supports_path(adapter, &entry.absolute_path) {
                    return None;
                }
                let source = fs::read_to_string(&entry.absolute_path).unwrap();
                let metadata = fs::metadata(&entry.absolute_path).unwrap();
                let mut extract = adapter.extract(&entry, &source);
                extract.file.content_hash =
                    Some(blake3::hash(source.as_bytes()).to_hex().to_string());
                extract.file.mtime_unix_seconds = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs() as i64);
                extract.file.size_bytes = Some(metadata.len() as i64);
                Some(extract)
            })
            .collect();
        store.persist_extract_results(&extracts).unwrap();
    }

    fn build_test_state(name: &str) -> (Arc<ServeState>, std::path::PathBuf) {
        let repo = prepare_fixture_copy(name);
        index_fixture(&repo);
        let paths = RuntimePaths {
            repo_root: repo.clone(),
            scope_dir: repo.join(".scope"),
            db_path: repo.join(".scope/index.db"),
        };
        (Arc::new(ServeState { paths }), repo)
    }

    async fn call(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn status_endpoint_returns_json_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/status").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "serve-status");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["stats"]["files"], 5);
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn deps_endpoint_returns_existing_envelope_shape() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/deps?file=src/lib.rs").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "deps");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/lib.rs");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn deps_endpoint_supports_transitive_closure_with_depth_limit() {
        let (state, repo) = build_test_state("ts_small");
        let app = build_router(state, false);
        let response = call(app, "/api/deps?file=src/index.ts&transitive=true&depth=2").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "deps");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/index.ts");
        assert_eq!(value["data"]["transitive"], true);
        assert_eq!(value["data"]["depth"], 2);
        assert_eq!(
            value["data"]["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "src/auth/index.ts",
                "src/utils/formatter.ts",
                "src/auth/aliases.ts",
                "src/auth/middleware.ts",
                "src/utils/logger.ts"
            ]
        );
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    #[ignore = "flaky in CI"]
    async fn audit_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("capability_audit");
        let app = build_router(state, false);
        let response = call(app, "/api/audit?capability=network").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "audit");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["capability"], "network");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn surface_diff_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("ts_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/surface/diff?before=src/auth/jwt.ts&after=src/auth/aliases.ts",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "surface-diff");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["before"], "src/auth/jwt.ts");
        assert_eq!(value["data"]["after"], "src/auth/aliases.ts");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn cycles_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/cycles").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cycles");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["summary"]["cycle_count"], 0);
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn report_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/report").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "report");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["metrics"]["total_files"]
            .as_u64()
            .is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn gate_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/gate?strict=true").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "gate");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["summary"]["total"]
            .as_u64()
            .is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn report_endpoint_missing_compare_snapshot_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/report?compare=missing").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "report");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "not_found");
        assert_eq!(value["data"]["message"], "snapshot not found: missing");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn gate_endpoint_missing_compare_snapshot_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/gate?compare=missing").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "gate");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "not_found");
        assert_eq!(value["data"]["message"], "snapshot not found: missing");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn gate_endpoint_invalid_strict_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/gate?strict=yes").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "gate");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("Failed to deserialize query string"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/query?expr=file%20%22src%2Flib.rs%22%20%7C%20.deps%20%7C%20count",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "ok");
        assert_eq!(
            value["data"]["input"],
            "file \"src/lib.rs\" | .deps | count"
        );
        assert!(value["data"]["result"].is_object());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_missing_expr_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/query").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("serve query requires `expr` or `exprs`"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_invalid_step_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/query?expr=file%20%22src%2Flib.rs%22%20%7C%20.impact",
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("unsupported query step `.impact`; supported steps are .deps, .reverse, .deps_transitive, .reverse_transitive, .symbols, .callers, .callees, unique, and count; plus .callers_transitive and .callees_transitive"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_supports_multiple_exprs_with_shared_bindings() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/query?exprs=let%20roots%20%3D%20file%20%22src%2Flib.rs%22%20%7C%20.deps%20%7C%20unique&exprs=%24roots%20%7C%20count",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["input"], "$roots | count");
        assert_eq!(value["data"]["result"]["number"], 3);
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_rejects_expr_and_exprs_together() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/query?expr=all-files%20%7C%20count&exprs=all-symbols%20%7C%20count",
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("serve query accepts either `expr` or `exprs`, but not both"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_supports_all_sources_and_reverse_step() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state.clone(), false);

        let all_files_response = call(app.clone(), "/api/query?expr=all-files%20%7C%20count").await;
        assert_eq!(all_files_response.status(), StatusCode::OK);
        let all_files_body = axum::body::to_bytes(all_files_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let all_files_value: Value = serde_json::from_slice(&all_files_body).unwrap();
        assert_eq!(all_files_value["command"], "query");
        assert_eq!(all_files_value["status"], "ok");
        assert_eq!(all_files_value["data"]["result"]["number"], 5);

        let all_symbols_response =
            call(app.clone(), "/api/query?expr=all-symbols%20%7C%20count").await;
        assert_eq!(all_symbols_response.status(), StatusCode::OK);
        let all_symbols_body = axum::body::to_bytes(all_symbols_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let all_symbols_value: Value = serde_json::from_slice(&all_symbols_body).unwrap();
        assert_eq!(all_symbols_value["command"], "query");
        assert_eq!(all_symbols_value["status"], "ok");
        assert!(
            all_symbols_value["data"]["result"]["number"]
                .as_u64()
                .expect("symbol count should be numeric")
                >= 4
        );

        let reverse_response = call(
            app,
            "/api/query?expr=file%20%22src%2Fparser.rs%22%20%7C%20.reverse%20%7C%20unique%20%7C%20count",
        )
        .await;
        assert_eq!(reverse_response.status(), StatusCode::OK);
        let reverse_body = axum::body::to_bytes(reverse_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let reverse_value: Value = serde_json::from_slice(&reverse_body).unwrap();
        assert_eq!(reverse_value["command"], "query");
        assert_eq!(reverse_value["status"], "ok");
        assert_eq!(reverse_value["data"]["result"]["number"], 2);

        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_supports_transitive_call_steps() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state.clone(), false);

        let callers_response = call(
            app.clone(),
            "/api/query?expr=symbol%20%22parser%3A%3Aparse%22%20%7C%20.callers_transitive%20%7C%20count",
        )
        .await;
        assert_eq!(callers_response.status(), StatusCode::OK);
        let callers_body = axum::body::to_bytes(callers_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let callers_value: Value = serde_json::from_slice(&callers_body).unwrap();
        assert_eq!(callers_value["command"], "query");
        assert_eq!(callers_value["status"], "ok");
        assert_eq!(callers_value["data"]["result"]["number"], 2);

        let callees_response = call(
            app,
            "/api/query?expr=symbol%20%22parser%3A%3Aparse%22%20%7C%20.callees_transitive%20%7C%20count",
        )
        .await;
        assert_eq!(callees_response.status(), StatusCode::OK);
        let callees_body = axum::body::to_bytes(callees_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let callees_value: Value = serde_json::from_slice(&callees_body).unwrap();
        assert_eq!(callees_value["command"], "query");
        assert_eq!(callees_value["status"], "ok");
        assert_eq!(callees_value["data"]["result"]["number"], 1);

        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn query_endpoint_unknown_binding_returns_json_error() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/query?expr=%24missing%20%7C%20count").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "query");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("unknown query binding `$missing`"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn unused_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/unused").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "unused");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["summary"]["exported_symbols"]
            .as_u64()
            .is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn explain_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/explain?target=parser::parse").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "explain");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "parser::parse");
        assert!(value["data"]["traversals"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn tree_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/tree?target=src/lib.rs").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "tree");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn entry_cone_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("capability_audit");
        let app = build_router(state, false);
        let response = call(app, "/api/entry/cone?target=src/workers/job.ts").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "entry-cone");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["entry"], "src/workers/job.ts");
        assert_eq!(value["data"]["result"]["summary"]["reachable_files"], 3);
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn entry_reaches_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("capability_audit");
        let app = build_router(state, false);
        let response = call(app, "/api/entry/reaches?target=src/shared/api.ts").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "entry-reaches");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "src/shared/api.ts");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn simulate_extract_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/simulate/extract?symbols=lib::parser&into=src/parser_extracted.rs",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "simulate-extract");
        assert_eq!(value["status"], "ok");
        assert_eq!(
            value["data"]["result"]["extraction"]["from_file"],
            "src/lib.rs"
        );
        assert_eq!(
            value["data"]["result"]["extraction"]["into_file"],
            "src/parser_extracted.rs"
        );
        let _ = fs::remove_dir_all(repo);
    }

    #[tokio::test]
    async fn simulate_extract_endpoint_rejects_empty_symbol_list() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/simulate/extract?symbols=,%20%20,&into=src/parser_extracted.rs",
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "simulate-extract");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("simulate extract requires at least one symbol"));
        let _ = fs::remove_dir_all(repo);
    }

    #[tokio::test]
    async fn cochange_endpoint_returns_expected_envelope_for_generated_git_fixture() {
        let repo = unique_temp_dir("cochange-serve");
        support::create_cochange_fixture_repo(&repo);

        index_fixture(&repo);
        let paths = RuntimePaths {
            repo_root: repo.clone(),
            scope_dir: repo.join(".scope"),
            db_path: repo.join(".scope/index.db"),
        };
        let app = build_router(Arc::new(ServeState { paths }), false);
        let response = call(app, "/api/cochange?target=src/parser.rs&days=10000").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
        assert_eq!(value["data"]["result"]["summary"]["target_commits"], 4);
        assert_eq!(value["data"]["result"]["files"][0]["path"], "src/utils.rs");
        assert_eq!(
            value["data"]["result"]["files"][1]["path"],
            "src/resolver.rs"
        );
        let _ = fs::remove_dir_all(repo);
    }

    #[tokio::test]
    async fn cochange_endpoint_rejects_invalid_days() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/cochange?target=src/lib.rs&days=0").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("cochange window days must be greater than 0"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn cochange_endpoint_rejects_invalid_top() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/cochange?target=src/lib.rs&top=0").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("top must be greater than 0"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn cochange_endpoint_rejects_invalid_min_shared_commits() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/cochange?target=src/lib.rs&min_shared_commits=0").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("min_shared_commits must be greater than 0"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn cochange_endpoint_rejects_missing_target() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/cochange?target=src/missing.rs").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("file not indexed: src/missing.rs"));
        let _ = fs::remove_dir_all(repo);
    }

    #[tokio::test]
    async fn symbols_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/symbols?file=src/lib.rs").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "symbols");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/lib.rs");
        assert!(value["data"]["symbols"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn calls_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/calls?symbol=lib::run").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "calls");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["symbol"], "lib::run");
        assert!(value["data"]["traversals"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn callers_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/callers?symbol=parser::parse").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "callers");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["symbol"], "parser::parse");
        assert!(value["data"]["traversals"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn impact_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/impact?target=parser::parse&change_type=body").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "impact");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "parser::parse");
        assert_eq!(value["data"]["change_type"], "body");
        assert!(value["data"]["summary"]["total"].as_u64().is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn why_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/why?from=src/lib.rs&to=src/parser.rs").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "why");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["from"], "src/lib.rs");
        assert_eq!(value["data"]["to"], "src/parser.rs");
        assert!(value["data"]["path"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn context_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(
            app,
            "/api/context?target=parser::parse&change_type=body&budget=400",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "context");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["change_type"], "body");
        assert_eq!(value["data"]["result"]["budget"], 400);
        assert!(value["data"]["result"]["summary"]["targets_count"]
            .as_u64()
            .is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn risk_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/risk?top=3").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "risk");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["top"], 3);
        assert!(value["data"]["result"]["summary"]["scored_files"]
            .as_u64()
            .is_some());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn stability_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/stability").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "stability");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["summary"]["flagged_count"]
            .as_u64()
            .is_some());
        assert!(value["data"]["result"]["files"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn surface_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/surface?target=src/lib.rs").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "surface");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/lib.rs");
        assert!(value["data"]["surface"]["symbols"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn entry_list_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("capability_audit");
        let app = build_router(state, false);
        let response = call(app, "/api/entry/list").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "entry-list");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["summary"]["entry_points"]
            .as_u64()
            .is_some());
        assert!(value["data"]["result"]["entry_points"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn entry_unreachable_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("capability_audit");
        let app = build_router(state, false);
        let response = call(app, "/api/entry/unreachable").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "entry-unreachable");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["unreachable_files"]
            .as_u64()
            .is_some());
        assert!(value["data"]["result"]["unreachable"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    #[ignore = "flaky in CI"]
    async fn snapshot_list_endpoint_returns_expected_envelope() {
        let (state, repo) = build_test_state("rust_small");
        let store = open_store(&state).unwrap();
        store.save_snapshot("baseline", None).unwrap();
        let app = build_router(state, false);
        let response = call(app, "/api/snapshot/list").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "snapshot-list");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["summary"]["snapshot_count"], 1);
        assert_eq!(value["data"]["result"]["snapshots"][0]["name"], "baseline");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn html_fallback_is_served_when_ui_enabled() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("scope serve"));
        assert!(html.contains("Bundled local API explorer for the indexed repository."));
        assert!(html.contains("/api/audit?capability=network"));
        assert!(
            html.contains("/api/surface/diff?before=src/auth/jwt.ts&amp;after=src/auth/aliases.ts")
        );
        assert!(html.contains("/api/tree?target=src/lib.rs"));
        assert!(html.contains(
            "/api/simulate/extract?symbols=lib::parser&amp;into=src/parser_extracted.rs"
        ));
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn status_endpoint_sets_json_content_type() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, false);
        let response = call(app, "/api/status").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "serve-status");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    #[ignore = "flaky in CI"]
    async fn snapshot_list_endpoint_sets_json_content_type() {
        let (state, repo) = build_test_state("rust_small");
        let store = open_store(&state).unwrap();
        store.save_snapshot("baseline", None).unwrap();
        let app = build_router(state, false);
        let response = call(app, "/api/snapshot/list").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "snapshot-list");
        fs::remove_dir_all(repo).unwrap();
    }

    #[tokio::test]
    async fn html_fallback_is_disabled_with_no_ui() {
        let (state, repo) = build_test_state("rust_small");
        let app = build_router(state, true);
        let response = call(app, "/").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["command"], "serve");
        assert_eq!(value["status"], "error");
        fs::remove_dir_all(repo).unwrap();
    }
}
