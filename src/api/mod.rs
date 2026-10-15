//! Localhost ontology HTTP API. Read + Action propose only; never silent writes.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use color_eyre::eyre::{Result, WrapErr, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::{ApiConfig, DEFAULT_API_BIND};
use crate::model::{AgentAction, ObjectView, Proposal, Workspace};
use crate::query::{
    SearchAroundView, list_objects, object_view_for_ref, propose_actions,
    resolution_suggestion_views, search_around_view,
};
use crate::storage::Store;

const OPENAPI_JSON: &str = include_str!("openapi.json");
const API_MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct ApiState {
    store: Arc<Mutex<Store>>,
    token: Arc<String>,
}

#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub bind: SocketAddr,
    pub token: String,
    pub ephemeral_token: bool,
    pub allow_non_loopback: bool,
}

impl ServeOptions {
    pub fn from_config(
        api: &ApiConfig,
        bind_override: Option<&str>,
        token_override: Option<&str>,
    ) -> Result<Self> {
        let bind_raw = bind_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(api.bind.trim())
            .trim();
        let bind_raw = if bind_raw.is_empty() {
            DEFAULT_API_BIND
        } else {
            bind_raw
        };
        let bind: SocketAddr = bind_raw
            .parse()
            .wrap_err_with(|| format!("invalid API bind address {bind_raw:?}"))?;
        let configured = token_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(api.token.trim());
        let (token, ephemeral_token) = if configured.is_empty() {
            (Uuid::new_v4().simple().to_string(), true)
        } else {
            (configured.to_string(), false)
        };
        Ok(Self {
            bind,
            token,
            ephemeral_token,
            allow_non_loopback: false,
        })
    }
}

/// Serves the ontology HTTP API until cancelled.
pub async fn serve(store: Store, options: ServeOptions) -> Result<()> {
    if !options.allow_non_loopback {
        require_loopback(options.bind)?;
    } else if !options.bind.ip().is_loopback() {
        tracing::warn!(
            bind = %options.bind,
            "API bind is not loopback; ontology data will be reachable beyond this host"
        );
    }
    let listener = TcpListener::bind(options.bind)
        .await
        .wrap_err_with(|| format!("could not bind {}", options.bind))?;
    let bound = listener
        .local_addr()
        .wrap_err("could not read bound address")?;
    if options.ephemeral_token {
        eprintln!("os-int API listening on http://{bound}");
        eprintln!("Authorization: Bearer {}", options.token);
        eprintln!("(ephemeral token for this process; set [api].token to pin it)");
    } else {
        eprintln!("os-int API listening on http://{bound}");
    }
    let router = router(store, options.token);
    axum::serve(listener, router)
        .await
        .wrap_err("API server failed")?;
    Ok(())
}

pub fn router(store: Store, token: impl Into<String>) -> Router {
    let state = ApiState {
        store: Arc::new(Mutex::new(store)),
        token: Arc::new(token.into()),
    };
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi))
        .route("/v1/investigations", get(list_investigations))
        .route("/v1/investigations/{id}", get(get_investigation))
        .route(
            "/v1/investigations/{id}/objects",
            get(list_investigation_objects),
        )
        .route(
            "/v1/investigations/{id}/objects/{reference}",
            get(get_object),
        )
        .route(
            "/v1/investigations/{id}/search-around",
            get(search_around_handler),
        )
        .route(
            "/v1/investigations/{id}/resolutions/suggest",
            get(suggest_handler),
        )
        .route("/v1/investigations/{id}/actions", post(propose_handler))
        .route("/v1/investigations/{id}/proposals", get(list_proposals))
        .layer(DefaultBodyLimit::max(API_MAX_BODY_BYTES))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn openapi() -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        OPENAPI_JSON,
    )
        .into_response()
}

async fn list_investigations(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigations = store.list_investigations().map_err(ApiError::internal)?;
    Ok(Json(
        serde_json::json!({ "investigations": investigations }),
    ))
}

async fn get_investigation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    Ok(Json(serde_json::json!({ "investigation": investigation })))
}

#[derive(Debug, Deserialize)]
struct ObjectListQuery {
    kind: Option<String>,
}

async fn list_investigation_objects(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<ObjectListQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let data = store
        .load_investigation(investigation.id)
        .map_err(ApiError::internal)?;
    let workspace = Workspace::new(data);
    let kind = query
        .kind
        .as_deref()
        .map(crate::query::parse_entity_kind_filter)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let objects = list_objects(&workspace, kind.as_ref());
    Ok(Json(serde_json::json!({ "objects": objects })))
}

async fn get_object(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((id, reference)): Path<(String, String)>,
) -> ApiResult<Json<ObjectView>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let view =
        object_view_for_ref(&store, investigation.id, &reference).map_err(ApiError::not_found)?;
    Ok(Json(view))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    seed: String,
    hops: Option<u32>,
    kind: Option<String>,
    link: Option<String>,
}

async fn search_around_handler(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchAroundView>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let data = store
        .load_investigation(investigation.id)
        .map_err(ApiError::internal)?;
    let workspace = Workspace::new(data);
    let kinds = query
        .kind
        .as_deref()
        .map(|value| {
            crate::query::parse_entity_kind_filter(value)
                .map(|kind| vec![kind])
                .map_err(ApiError::bad_request)
        })
        .transpose()?
        .unwrap_or_default();
    let links = query
        .link
        .as_deref()
        .map(|value| vec![value.to_string()])
        .unwrap_or_default();
    let view = search_around_view(
        &workspace,
        &query.seed,
        query.hops.unwrap_or(crate::model::DEFAULT_SEARCH_HOPS),
        &kinds,
        &links,
    )
    .map_err(ApiError::not_found)?;
    Ok(Json(view))
}

async fn suggest_handler(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let data = store
        .load_investigation(investigation.id)
        .map_err(ApiError::internal)?;
    let workspace = Workspace::new(data);
    Ok(Json(serde_json::json!({
        "suggestions": resolution_suggestion_views(&workspace)
    })))
}

#[derive(Debug, Deserialize)]
struct ProposeBody {
    actions: Vec<AgentAction>,
}

async fn propose_handler(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Result<Json<ProposeBody>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    authorize(&headers, &state.token)?;
    let Json(body) = body.map_err(ApiError::malformed_json)?;
    let mut store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let proposals = propose_actions(&mut store, investigation.id, "api", body.actions)
        .map_err(ApiError::forbidden)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "pending",
            "proposals": proposals,
        })),
    ))
}

async fn list_proposals(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Proposal>>> {
    authorize(&headers, &state.token)?;
    let store = state.store.lock().await;
    let investigation = store
        .require_investigation(&id)
        .map_err(ApiError::not_found)?;
    let data = store
        .load_investigation(investigation.id)
        .map_err(ApiError::internal)?;
    Ok(Json(data.proposals))
}

fn authorize(headers: &HeaderMap, expected: &str) -> ApiResult<()> {
    let provided = bearer_token(headers).ok_or_else(ApiError::unauthorized)?;
    if !tokens_match(expected, provided) {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    {
        return value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "));
    }
    headers
        .get("x-osint-token")
        .and_then(|value| value.to_str().ok())
}

fn tokens_match(expected: &str, provided: &str) -> bool {
    let left = Sha256::digest(expected.as_bytes());
    let right = Sha256::digest(provided.as_bytes());
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

type ApiResult<T> = std::result::Result<T, ApiError>;

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized".into(),
        }
    }

    fn not_found(error: color_eyre::eyre::Report) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: error.to_string(),
        }
    }

    fn forbidden(error: color_eyre::eyre::Report) -> Self {
        let message = error.to_string();
        let status = if message.contains("cannot modify") || message.contains("reader") {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::BAD_REQUEST
        };
        Self { status, message }
    }

    fn internal(error: color_eyre::eyre::Report) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }

    fn bad_request(error: color_eyre::eyre::Report) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }

    fn malformed_json(rejection: JsonRejection) -> Self {
        let status = rejection.status();
        let status = if status == StatusCode::PAYLOAD_TOO_LARGE {
            status
        } else {
            StatusCode::BAD_REQUEST
        };
        Self {
            status,
            message: rejection.body_text(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

/// Refuses a non-loopback bind unless `--allow-non-loopback` was passed.
pub fn require_loopback(bind: SocketAddr) -> Result<()> {
    if !bind.ip().is_loopback() {
        bail!(
            "API bind {bind} is not loopback; pass --allow-non-loopback if you intend to expose the ontology API beyond this host"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EntityKind, EvidenceBatch, Identity, NewSource, Provenance, Role, SourceKind, relation,
    };
    use crate::storage::{ApplyContext, Store};

    async fn spawn_api(store: Store, token: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let router = router(store, token);
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });
        format!("http://{addr}")
    }

    fn seeded_store() -> Store {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("API Case", "").expect("case");
        let mut batch = EvidenceBatch::new();
        let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS");
        source.provenance = Provenance::Collected;
        let key = batch.add_source(source);
        let domain = batch.entity(EntityKind::Domain, "example.com", Some(key));
        let ip = batch.entity(EntityKind::Ip, "192.0.2.10", Some(key));
        batch.relate(&domain, relation::RESOLVES_TO, &ip, Some(key));
        store
            .apply_batch(
                ApplyContext {
                    investigation_id: investigation.id,
                    page_id: None,
                    job_id: None,
                },
                &batch,
            )
            .expect("apply");
        store
    }

    #[tokio::test]
    async fn unauthorized_object_view_fails() {
        let store = seeded_store();
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/v1/investigations"))
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        let health = client
            .get(format!("{base}/health"))
            .send()
            .await
            .expect("health");
        assert_eq!(health.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn object_view_and_propose_round_trip() {
        let store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let view = client
            .get(format!(
                "{base}/v1/investigations/{}/objects/E-1",
                investigation.investigation.id
            ))
            .header("Authorization", "Bearer secret-token")
            .send()
            .await
            .expect("object");
        assert_eq!(view.status(), reqwest::StatusCode::OK);
        let body: ObjectView =
            serde_json::from_slice(&view.bytes().await.expect("bytes")).expect("object view json");
        assert_eq!(body.short_ref, "E-1");
        assert_eq!(body.object_type, "domain");
        let payload = serde_json::json!({
            "actions": [{
                "action": "ADD_FINDING",
                "title": "API proposal",
                "details": "must stay pending"
            }]
        });
        let propose = client
            .post(format!(
                "{base}/v1/investigations/{}/actions",
                investigation.investigation.id
            ))
            .header("Authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .expect("propose");
        assert_eq!(propose.status(), reqwest::StatusCode::ACCEPTED);
        let proposed: serde_json::Value =
            serde_json::from_slice(&propose.bytes().await.expect("bytes")).expect("propose json");
        assert_eq!(proposed["status"], "pending");
        assert_eq!(proposed["proposals"][0]["status"], "pending");
    }

    #[test]
    fn empty_token_mints_ephemeral() {
        let options = ServeOptions::from_config(&ApiConfig::default(), None, None).expect("opts");
        assert!(options.ephemeral_token);
        assert!(!options.token.is_empty());
        assert!(options.bind.ip().is_loopback());
        assert!(!options.allow_non_loopback);
    }

    #[test]
    fn non_loopback_bind_is_refused_without_flag() {
        let bind: SocketAddr = "0.0.0.0:8743".parse().expect("addr");
        let error = require_loopback(bind).expect_err("must refuse");
        assert!(error.to_string().contains("loopback"));
        assert!(error.to_string().contains("allow-non-loopback"));
    }

    #[tokio::test]
    async fn non_loopback_serve_fails_without_flag() {
        let store = Store::open_in_memory().expect("store");
        let options = ServeOptions {
            bind: "0.0.0.0:0".parse().expect("addr"),
            token: "secret".into(),
            ephemeral_token: false,
            allow_non_loopback: false,
        };
        let error = serve(store, options).await.expect_err("must refuse");
        assert!(error.to_string().contains("loopback"));
    }

    #[tokio::test]
    async fn wrong_token_is_unauthorized() {
        let store = seeded_store();
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/v1/investigations"))
            .header("Authorization", "Bearer wrong-token")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn x_osint_token_header_authenticates() {
        let store = seeded_store();
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/v1/investigations"))
            .header("X-Osint-Token", "secret-token")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn reader_post_is_forbidden() {
        let mut store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        store
            .add_member(investigation.investigation.id, "carol", Role::Reader)
            .expect("member");
        store.set_identity(Identity {
            user_id: "carol".into(),
            machine_name: "desk-c".into(),
        });
        let id = investigation.investigation.id;
        let base = spawn_api(store, "secret-token").await;
        let payload = serde_json::json!({
            "actions": [{
                "action": "ADD_FINDING",
                "title": "reader must not write",
                "details": ""
            }]
        });
        let response = reqwest::Client::new()
            .post(format!("{base}/v1/investigations/{id}/actions"))
            .header("Authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn oversized_body_is_payload_too_large() {
        let store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        let base = spawn_api(store, "secret-token").await;
        let body = "x".repeat(API_MAX_BODY_BYTES + 1);
        let response = reqwest::Client::new()
            .post(format!(
                "{base}/v1/investigations/{}/actions",
                investigation.investigation.id
            ))
            .header("Authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn malformed_json_body_returns_json_400() {
        let store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        let base = spawn_api(store, "secret-token").await;
        let response = reqwest::Client::new()
            .post(format!(
                "{base}/v1/investigations/{}/actions",
                investigation.investigation.id
            ))
            .header("Authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body("{not json")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_slice(&response.bytes().await.expect("bytes")).expect("json");
        assert!(
            body["error"].as_str().is_some_and(|text| !text.is_empty()),
            "{body}"
        );
    }

    #[tokio::test]
    async fn concurrent_reads_and_propose_stay_consistent() {
        let store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        let id = investigation.investigation.id;
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let list = format!("{base}/v1/investigations");
        let objects = format!("{base}/v1/investigations/{id}/objects");
        let around = format!("{base}/v1/investigations/{id}/search-around?seed=E-1&hops=1");
        let actions = format!("{base}/v1/investigations/{id}/actions");
        let payload = serde_json::json!({
            "actions": [{
                "action": "ADD_FINDING",
                "title": "concurrent propose",
                "details": "pending only"
            }]
        });
        let (a, b, c, d) = tokio::join!(
            client
                .get(&list)
                .header("Authorization", "Bearer secret-token")
                .send(),
            client
                .get(&objects)
                .header("Authorization", "Bearer secret-token")
                .send(),
            client
                .get(&around)
                .header("Authorization", "Bearer secret-token")
                .send(),
            client
                .post(&actions)
                .header("Authorization", "Bearer secret-token")
                .header("content-type", "application/json")
                .body(payload.to_string())
                .send(),
        );
        assert_eq!(a.expect("list").status(), reqwest::StatusCode::OK);
        assert_eq!(b.expect("objects").status(), reqwest::StatusCode::OK);
        assert_eq!(c.expect("search-around").status(), reqwest::StatusCode::OK);
        let propose = d.expect("propose");
        assert_eq!(propose.status(), reqwest::StatusCode::ACCEPTED);
        let body: serde_json::Value =
            serde_json::from_slice(&propose.bytes().await.expect("bytes")).expect("json");
        assert_eq!(body["status"], "pending");
    }

    #[tokio::test]
    async fn unknown_kind_filter_is_bad_request() {
        let store = seeded_store();
        let investigation = store
            .list_investigations()
            .expect("list")
            .into_iter()
            .next()
            .expect("one");
        let id = investigation.investigation.id;
        let base = spawn_api(store, "secret-token").await;
        let client = reqwest::Client::new();
        let objects = client
            .get(format!("{base}/v1/investigations/{id}/objects?kind=domian"))
            .header("Authorization", "Bearer secret-token")
            .send()
            .await
            .expect("objects");
        assert_eq!(objects.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_slice(&objects.bytes().await.expect("bytes")).expect("json");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|text| text.contains("unknown entity kind")),
            "{body}"
        );
        let around = client
            .get(format!(
                "{base}/v1/investigations/{id}/search-around?seed=E-1&kind=domian"
            ))
            .header("Authorization", "Bearer secret-token")
            .send()
            .await
            .expect("search-around");
        assert_eq!(around.status(), reqwest::StatusCode::BAD_REQUEST);
    }
}
