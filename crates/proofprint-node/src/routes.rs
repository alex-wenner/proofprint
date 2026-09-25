//! HTTP surface. Handlers parse, hand off to [`Node`], and encode.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::multipart::MultipartRejection;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use proofprint_core::{BlobId, BlobRef, RecordId, SignedRecord};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::error::NodeError;
use crate::node::{
    Draft, Node, Proof, Published, RecordDetail, RecordFilter, RecordPage, Status, Tree,
    VerifyReport,
};

/// Largest accepted record body. Canonical records are capped at 1 MiB by core.
const RECORD_LIMIT: usize = 2 * 1024 * 1024;

type Shared = State<Arc<Node>>;
type Reply<T> = Result<Json<T>, NodeError>;

/// Static configuration of the router.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Directory holding a built explorer; served for every non-API path.
    pub ui: PathBuf,
    /// Largest accepted artifact upload, in bytes.
    pub upload_limit: u64,
    /// Require a loopback `Host` header. Set when listening on loopback, to
    /// defeat DNS rebinding. Cross-origin writes are refused either way.
    pub loopback_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui: PathBuf::from("explorer/dist"),
            upload_limit: 4 << 30,
            loopback_only: true,
        }
    }
}

pub fn router(node: Arc<Node>, settings: Settings) -> Router {
    let upload_limit = usize::try_from(settings.upload_limit).unwrap_or(usize::MAX);
    let index = settings.ui.join("index.html");
    let loopback_only = settings.loopback_only;
    Router::new()
        .route("/api/status", get(status))
        .route(
            "/api/records",
            get(records)
                .post(append)
                .layer(DefaultBodyLimit::max(RECORD_LIMIT)),
        )
        .route("/api/records/{id}", get(record))
        .route("/api/records/{id}/tree", get(tree))
        .route("/api/records/{id}/proof", get(proof))
        .route("/api/records/{id}/verify", post(verify))
        .route(
            "/api/publish",
            post(publish).layer(DefaultBodyLimit::max(RECORD_LIMIT)),
        )
        .route(
            "/api/artifacts",
            post(upload).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/api/artifacts/{id}", get(download))
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .fallback_service(ServeDir::new(&settings.ui).fallback(ServeFile::new(index)))
        .layer(axum::middleware::from_fn(move |request, next| {
            crate::guard::layer(loopback_only, request, next)
        }))
        .layer(TraceLayer::new_for_http())
        .with_state(node)
}

async fn api_not_found() -> NodeError {
    NodeError::NotFound
}

async fn status(State(node): Shared) -> Reply<Status> {
    node.run(Node::status).await.map(Json)
}

async fn records(
    State(node): Shared,
    query: Result<Query<RecordFilter>, QueryRejection>,
) -> Reply<RecordPage> {
    let Query(filter) = query?;
    node.run(move |node| node.records(&filter)).await.map(Json)
}

async fn record(State(node): Shared, Path(id): Path<String>) -> Reply<RecordDetail> {
    let id = RecordId::parse(&id)?;
    node.run(move |node| node.record(&id)).await.map(Json)
}

async fn tree(State(node): Shared, Path(id): Path<String>) -> Reply<Tree> {
    let id = RecordId::parse(&id)?;
    node.run(move |node| node.tree(&id)).await.map(Json)
}

async fn proof(State(node): Shared, Path(id): Path<String>) -> Reply<Proof> {
    let id = RecordId::parse(&id)?;
    node.run(move |node| node.proof(&id)).await.map(Json)
}

async fn verify(State(node): Shared, Path(id): Path<String>) -> Reply<VerifyReport> {
    let id = RecordId::parse(&id)?;
    node.run(move |node| node.verify(&id)).await.map(Json)
}

async fn append(
    State(node): Shared,
    body: Result<Json<SignedRecord>, JsonRejection>,
) -> Reply<Published> {
    let Json(signed) = body?;
    node.run(move |node| node.append(signed)).await.map(Json)
}

async fn publish(
    State(node): Shared,
    body: Result<Json<Draft>, JsonRejection>,
) -> Reply<Published> {
    let Json(draft) = body?;
    node.run(move |node| node.publish(draft)).await.map(Json)
}

/// One file per request. The part's filename becomes the attachment name.
async fn upload(
    State(node): Shared,
    body: Result<Multipart, MultipartRejection>,
) -> Reply<BlobRef> {
    let mut multipart = body?;
    let store = node.run(Node::blobs).await?;
    let mut field = multipart
        .next_field()
        .await?
        .ok_or_else(|| NodeError::bad_request("expected one file part"))?;
    let name = field.file_name().unwrap_or("artifact").to_owned();
    let media = field.content_type().map(str::to_owned);

    let staging = store.staging_file()?;
    let mut file = tokio::fs::File::from_std(staging.reopen().map_err(NodeError::internal)?);
    while let Some(chunk) = field.chunk().await? {
        file.write_all(&chunk).await.map_err(NodeError::internal)?;
    }
    file.flush().await.map_err(NodeError::internal)?;
    drop(file);
    drop(field);

    if multipart.next_field().await?.is_some() {
        return Err(NodeError::bad_request("upload one file per request"));
    }
    let reference = tokio::task::spawn_blocking(move || store.put_staged(staging, name, media))
        .await
        .map_err(NodeError::internal)??;
    Ok(Json(reference))
}

async fn download(State(node): Shared, Path(id): Path<String>) -> Result<Response, NodeError> {
    let id = BlobId::parse(&id)?;
    let file = node
        .run(move |node| Ok(node.blobs()?.open_reader(&id)?))
        .await?;
    let size = file.metadata().map_err(NodeError::internal)?.len();
    let body = Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file)));
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
        (header::CONTENT_LENGTH, size.to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}.bin\"", id.to_hex()),
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
    ];
    Ok((StatusCode::OK, headers, body).into_response())
}
