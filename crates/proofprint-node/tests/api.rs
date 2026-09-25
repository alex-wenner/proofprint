//! Router tests. Requests go straight to the tower service: no socket, no browser.

use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use proofprint_core::{BlobId, BlobRef, KeyPair, Ledger, Record, RecordId, SignedRecord};
use proofprint_node::{demo, router, Node, Settings};
use serde::Serialize;
use serde_json::{json, Value};
use tower::ServiceExt;

struct TestNode {
    dir: tempfile::TempDir,
    node: Arc<Node>,
    router: Router,
    keys: KeyPair,
}

impl TestNode {
    fn new() -> Self {
        Self::build(true, false)
    }

    fn without_key() -> Self {
        Self::build(false, false)
    }

    fn seeded() -> Self {
        Self::build(true, true)
    }

    fn build(with_key: bool, seed: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let keys = KeyPair::generate();
        let mut ledger = Ledger::open(dir.path().join("data")).unwrap();
        if seed {
            assert_eq!(demo::seed(&mut ledger, &keys).unwrap().len(), 8);
            assert!(demo::seed(&mut ledger, &keys).unwrap().is_empty());
        }
        let node = Arc::new(Node::new(ledger, with_key.then(|| keys.clone())));
        let settings = Settings {
            ui: dir.path().join("ui"),
            ..Settings::default()
        };
        let router = router(node.clone(), settings);
        Self {
            dir,
            node,
            router,
            keys,
        }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, body)
    }

    async fn json(&self, request: Request<Body>) -> (StatusCode, Value) {
        let (status, _, body) = self.send(request).await;
        let value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| panic!("non-JSON body: {}", String::from_utf8_lossy(&body)));
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.json(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn post(&self, path: &str) -> (StatusCode, Value) {
        self.json(Request::post(path).body(Body::empty()).unwrap())
            .await
    }

    async fn post_json(&self, path: &str, body: &impl Serialize) -> (StatusCode, Value) {
        let request = Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        self.json(request).await
    }

    async fn upload(&self, files: &[(&str, &str, &[u8])]) -> (StatusCode, Value) {
        let boundary = "test-boundary";
        let mut body = Vec::new();
        for (name, media, bytes) in files {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
                     filename=\"{name}\"\r\nContent-Type: {media}\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let request = Request::post("/api/artifacts")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.json(request).await
    }

    fn signed(&self, kind: &str, payload: Value, parents: &[RecordId]) -> SignedRecord {
        Record::new(kind, payload, self.keys.signer())
            .with_parents(parents.iter().copied())
            .sign(&self.keys)
            .unwrap()
    }

    async fn append(&self, signed: &SignedRecord) -> RecordId {
        let (status, body) = self.post_json("/api/records", signed).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["id"].as_str().unwrap().parse().unwrap()
    }
}

fn unknown_record() -> String {
    RecordId::from_bytes(b"unknown").to_string()
}

#[tokio::test]
async fn status_of_an_empty_ledger() {
    let t = TestNode::new();
    let (status, body) = t.get("/api/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records"], 0);
    assert_eq!(body["artifacts"], 0);
    assert!(body["root"].is_null());
    assert_eq!(body["signer"], t.keys.signer());
}

#[tokio::test]
async fn signed_records_round_trip() {
    let t = TestNode::new();
    let signed = t.signed("example.note/v1", json!({"title": "first"}), &[]);
    let id = t.append(&signed).await;

    let (status, again) = t.post_json("/api/records", &signed).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["position"], 0, "re-publishing is a no-op");

    let (_, page) = t.get("/api/records").await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["records"][0]["id"], id.to_string());
    assert_eq!(page["records"][0]["label"], "first");

    let (status, detail) = t.get(&format!("/api/records/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["position"], 0);
    assert_eq!(detail["signed"]["sig"], signed.sig);
    assert_eq!(detail["signed"]["record"]["payload"]["title"], "first");
    assert_eq!(detail["children"], json!([]));
    assert_eq!(
        detail["canonical"].as_str().unwrap().as_bytes(),
        signed.canonical_bytes().unwrap()
    );

    let (status, proof) = t.get(&format!("/api/records/{id}/proof")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(proof["valid"], true);
    assert_eq!(proof["tree_size"], 1);
    assert_eq!(proof["root"], page["root"]);

    let (status, report) = t.post(&format!("/api/records/{id}/verify")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["signature"], true);
    assert_eq!(report["inclusion"], true);
    assert_eq!(report["complete"], true);
}

/// Browsers parse JSON numbers as doubles. An envelope rebuilt from the
/// canonical text, not from parsed values, still verifies elsewhere.
#[tokio::test]
async fn canonical_text_rebuilds_an_envelope_without_losing_integers() {
    let t = TestNode::new();
    let big = 9_007_199_254_740_993_u64;
    let signed = t.signed("example.note/v1", json!({ "count": big }), &[]);
    let id = t.append(&signed).await;
    let (_, detail) = t.get(&format!("/api/records/{id}")).await;
    let canonical = detail["canonical"].as_str().unwrap();
    assert!(canonical.contains("9007199254740993"));

    let envelope = format!(r#"{{"record":{canonical},"sig":"{}"}}"#, signed.sig);
    let rebuilt: SignedRecord = serde_json::from_str(&envelope).unwrap();
    assert_eq!(rebuilt.verify().unwrap(), id);
}

#[tokio::test]
async fn listing_filters_by_kind_and_signer_and_pages() {
    let t = TestNode::new();
    let other = KeyPair::generate();
    let first = t.append(&t.signed("example.note/v1", json!({}), &[])).await;
    t.append(&t.signed("ml.evaluation/v1", json!({}), &[first]))
        .await;
    let foreign = Record::new("example.note/v1", json!({}), other.signer())
        .sign(&other)
        .unwrap();
    t.append(&foreign).await;

    let (_, page) = t.get("/api/records?kind=example.note/v1").await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["records"].as_array().unwrap().len(), 2);

    let (_, page) = t
        .get(&format!("/api/records?signer={}", other.signer()))
        .await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["records"][0]["position"], 2);

    let (_, page) = t.get("/api/records?offset=1&limit=1").await;
    assert_eq!(page["total"], 3);
    assert_eq!(page["offset"], 1);
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
    assert_eq!(page["records"][0]["position"], 1);

    let (_, page) = t.get("/api/records?kind=nope/v1").await;
    assert_eq!(page["total"], 0);
    assert_eq!(page["records"], json!([]));

    let (status, _) = t.get("/api/records?offset=notanumber").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn malformed_and_unknown_ids() {
    let t = TestNode::new();
    t.append(&t.signed("example.note/v1", json!({}), &[])).await;

    for path in [
        "/api/records/nope",
        "/api/records/nope/proof",
        "/api/records/nope/tree",
        "/api/artifacts/nope",
    ] {
        let (status, body) = t.get(path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert!(body["error"].is_string(), "{path}");
    }
    let (status, _) = t.post("/api/records/nope/verify").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let unknown = unknown_record();
    for path in [
        format!("/api/records/{unknown}"),
        format!("/api/records/{unknown}/proof"),
        format!("/api/records/{unknown}/tree"),
        format!("/api/artifacts/{}", BlobId::from_bytes(b"unknown")),
    ] {
        let (status, body) = t.get(&path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(body["error"].is_string(), "{path}");
    }
    let (status, _) = t.post(&format!("/api/records/{unknown}/verify")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tampered_records_are_refused() {
    let t = TestNode::new();
    let mut signed = t.signed("example.note/v1", json!({"n": 1}), &[]);
    signed.record.payload = json!({"n": 2});
    let (status, body) = t.post_json("/api/records", &signed).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "signature verification failed");
    let (_, status) = t.get("/api/status").await;
    assert_eq!(status["records"], 0);
}

#[tokio::test]
async fn missing_parents_are_refused() {
    let t = TestNode::new();
    let orphan = t.signed(
        "example.note/v1",
        json!({}),
        &[RecordId::from_bytes(b"ghost")],
    );
    let (status, body) = t.post_json("/api/records", &orphan).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .starts_with("parent record not found"));
}

#[tokio::test]
async fn bodies_that_are_not_records_are_refused() {
    let t = TestNode::new();
    let (status, body) = t.post_json("/api/records", &json!({"record": "no"})).await;
    assert!(status.is_client_error(), "{status} {body}");

    let big = json!({"record": {"payload": {"filler": "x".repeat(3 * 1024 * 1024)}}});
    let (status, _) = t.post_json("/api/records", &big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let request = Request::post("/api/records")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("hello"))
        .unwrap();
    let (status, _) = t.json(request).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn artifacts_upload_download_and_verify() {
    let t = TestNode::new();
    let bytes: &[u8] = b"demonstration weights";
    let (status, reference) = t
        .upload(&[("weights", "application/octet-stream", bytes)])
        .await;
    assert_eq!(status, StatusCode::OK, "{reference}");
    let blob: BlobRef = serde_json::from_value(reference).unwrap();
    assert_eq!(blob.name, "weights");
    assert_eq!(blob.size, bytes.len() as u64);
    assert_eq!(blob.media.as_deref(), Some("application/octet-stream"));
    assert_eq!(blob.blob, BlobId::from_bytes(bytes));

    let (status, again) = t.upload(&[("weights", "text/plain", bytes)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["blob"], blob.blob.to_string());
    assert_eq!(t.node.status().unwrap().artifacts, 1);

    let request = Request::get(format!("/api/artifacts/{}", blob.blob))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = t.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], bytes);
    assert_eq!(headers[header::CONTENT_LENGTH], bytes.len().to_string());
    assert_eq!(headers[header::CONTENT_TYPE], "application/octet-stream");

    let record = Record::new("ml.model/v1", json!({"name": "m"}), t.keys.signer())
        .with_blob(blob.clone())
        .sign(&t.keys)
        .unwrap();
    let id = t.append(&record).await;
    let (_, detail) = t.get(&format!("/api/records/{id}")).await;
    assert_eq!(detail["artifacts"][0]["available"], true);
    assert_eq!(detail["artifacts"][0]["reference"]["name"], "weights");

    let verify = format!("/api/records/{id}/verify");
    let (_, report) = t.post(&verify).await;
    assert_eq!(report["complete"], true);
    assert_eq!(report["verified"], json!(["weights"]));
    assert_eq!(report["verified_bytes"], bytes.len());

    let path = t.node.blobs().unwrap().path_for(&blob.blob);
    fs::write(&path, b"replaced").unwrap();
    let (status, report) = t.post(&verify).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["consistent"], false);
    assert_eq!(report["complete"], false);
    assert_eq!(report["corrupt"][0][0], "weights");

    fs::remove_file(&path).unwrap();
    let (_, report) = t.post(&verify).await;
    assert_eq!(report["consistent"], true);
    assert_eq!(report["complete"], false);
    assert_eq!(report["missing"], json!(["weights"]));
    let (_, detail) = t.get(&format!("/api/records/{id}")).await;
    assert_eq!(detail["artifacts"][0]["available"], false);
    let (status, _) = t.get(&format!("/api/artifacts/{}", blob.blob)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn uploads_take_exactly_one_file() {
    let t = TestNode::new();
    let (status, _) = t
        .upload(&[("a", "text/plain", b"a"), ("b", "text/plain", b"b")])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = t.upload(&[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(t.node.status().unwrap().artifacts, 0);

    let request = Request::post("/api/artifacts")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, body) = t.json(request).await;
    assert!(status.is_client_error(), "{status} {body}");
}

#[tokio::test]
async fn drafts_are_signed_by_the_node() {
    let t = TestNode::new();
    let parent = t
        .append(&t.signed("ml.dataset/v1", json!({"name": "d"}), &[]))
        .await;
    let draft = json!({
        "kind": "ml.training-run/v1",
        "payload": {"run_id": "r1"},
        "parents": [parent],
        "created": "2026-01-15T12:00:00Z"
    });
    let (status, published) = t.post_json("/api/publish", &draft).await;
    assert_eq!(status, StatusCode::OK, "{published}");
    assert_eq!(published["position"], 1);

    let id = published["id"].as_str().unwrap();
    let (_, detail) = t.get(&format!("/api/records/{id}")).await;
    let record = &detail["signed"]["record"];
    assert_eq!(record["signer"], t.keys.signer());
    assert_eq!(record["created"], "2026-01-15T12:00:00Z");
    assert_eq!(record["parents"][0], parent.to_string());
    let signed: SignedRecord = serde_json::from_value(detail["signed"].clone()).unwrap();
    assert!(signed.verify().is_ok());

    let (status, _) = t
        .post_json("/api/publish", &json!({"kind": "example.note/v1"}))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "payload defaults to an empty object"
    );

    let (status, body) = t
        .post_json("/api/publish", &json!({"kind": "Not A Kind"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, _) = t
        .post_json(
            "/api/publish",
            &json!({"kind": "example.note/v1", "payload": [1]}),
        )
        .await;
    assert!(status.is_client_error());

    let (status, body) = t
        .post_json(
            "/api/publish",
            &json!({"kind": "example.note/v1", "parents": [unknown_record()]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn drafts_need_a_signing_key() {
    let t = TestNode::without_key();
    let (_, status) = t.get("/api/status").await;
    assert!(status["signer"].is_null());

    let (status, body) = t
        .post_json("/api/publish", &json!({"kind": "example.note/v1"}))
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("signing is disabled"));

    t.append(&t.signed("example.note/v1", json!({}), &[])).await;
}

#[tokio::test]
async fn tree_lists_ancestors_and_descendants() {
    let t = TestNode::new();
    let root = t
        .append(&t.signed("example.note/v1", json!({"title": "root"}), &[]))
        .await;
    let left = t
        .append(&t.signed("example.note/v1", json!({"title": "left"}), &[root]))
        .await;
    let right = t
        .append(&t.signed("example.note/v1", json!({"title": "right"}), &[root]))
        .await;
    let merge = t
        .append(&t.signed("example.note/v1", json!({"title": "merge"}), &[left, right]))
        .await;

    let (status, tree) = t.get(&format!("/api/records/{left}/tree")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tree["record"]["label"], "left");
    assert_eq!(tree["ancestors"][0]["id"], root.to_string());
    assert_eq!(tree["descendants"][0]["id"], merge.to_string());
    assert_eq!(tree["missing"], json!([]));

    let (_, tree) = t.get(&format!("/api/records/{root}/tree")).await;
    assert_eq!(tree["descendants"].as_array().unwrap().len(), 3);

    let (_, detail) = t.get(&format!("/api/records/{root}")).await;
    assert_eq!(detail["children"], json!([left, right]));
}

#[tokio::test]
async fn unknown_api_routes_answer_json_404() {
    let t = TestNode::new();
    for (method, path) in [
        (Method::GET, "/api"),
        (Method::GET, "/api/nothing"),
        (Method::DELETE, "/api/records/x/y/z"),
    ] {
        let request = Request::builder()
            .method(method.clone())
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let (status, body) = t.json(request).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(body["error"], "not found");
    }
    let request = Request::put("/api/records").body(Body::empty()).unwrap();
    let (status, _, _) = t.send(request).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn explorer_build_is_served_with_index_fallback() {
    let t = TestNode::new();
    let request = Request::get("/").body(Body::empty()).unwrap();
    let (status, _, _) = t.send(request).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing built yet");

    let ui = t.dir.path().join("ui");
    fs::create_dir_all(ui.join("assets")).unwrap();
    fs::write(
        ui.join("index.html"),
        "<!doctype html><title>Explorer</title>",
    )
    .unwrap();
    fs::write(ui.join("assets/app.js"), "console.log(1)").unwrap();

    for path in ["/", "/index.html", "/records/anything"] {
        let request = Request::get(path).body(Body::empty()).unwrap();
        let (status, _, body) = t.send(request).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.starts_with(b"<!doctype html>"), "{path}");
    }
    let request = Request::get("/assets/app.js").body(Body::empty()).unwrap();
    let (status, headers, body) = t.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"console.log(1)");
    assert!(headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .contains("javascript"));

    let (status, page) = t.get("/api/records").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn demo_seed_builds_a_two_signer_graph() {
    let t = TestNode::seeded();
    let (_, status) = t.get("/api/status").await;
    assert_eq!(status["records"], 8);
    assert_eq!(status["artifacts"], 2);

    let (_, page) = t.get("/api/records?limit=1000").await;
    let records = page["records"].as_array().unwrap();
    let signers: HashSet<&str> = records
        .iter()
        .map(|r| r["signer"].as_str().unwrap())
        .collect();
    assert_eq!(signers.len(), 2);
    let labels: Vec<&str> = records
        .iter()
        .map(|r| r["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "demo-corpus",
            "demo-run-1",
            "step 600",
            "step 1200",
            "demo-model",
            "demo-benchmark",
            "demo-benchmark",
            "re-evaluation",
        ]
    );

    let (_, evaluations) = t.get("/api/records?kind=ml.evaluation/v1").await;
    assert_eq!(evaluations["total"], 2);

    for record in records {
        let id = record["id"].as_str().unwrap();
        let (status, report) = t.post(&format!("/api/records/{id}/verify")).await;
        assert_eq!(status, StatusCode::OK, "{id}");
        assert_eq!(report["complete"], true, "{id}: {report}");
    }

    let last = records.last().unwrap()["id"].as_str().unwrap();
    let (_, tree) = t.get(&format!("/api/records/{last}/tree")).await;
    assert_eq!(tree["record"]["kind"], "attestation.verification/v1");
    assert_eq!(tree["ancestors"].as_array().unwrap().len(), 7);
    assert_eq!(tree["descendants"], json!([]));
}

#[tokio::test]
async fn browser_borne_writes_are_refused() {
    let node = TestNode::new();
    let cross_site = Request::post("/api/publish")
        .header(header::HOST, "127.0.0.1:4780")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(node.send(cross_site).await.0, StatusCode::FORBIDDEN);

    let rebound = Request::get("/api/status")
        .header(header::HOST, "attacker.example:4780")
        .body(Body::empty())
        .unwrap();
    assert_eq!(node.send(rebound).await.0, StatusCode::FORBIDDEN);

    let local = Request::get("/api/status")
        .header(header::HOST, "localhost:4780")
        .body(Body::empty())
        .unwrap();
    assert_eq!(node.send(local).await.0, StatusCode::OK);
}
