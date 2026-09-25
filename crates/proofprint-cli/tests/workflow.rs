use std::process::{Command, Output};

fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_proofprint"))
        .env_remove("PROOFPRINT_KEY")
        .env_remove("PROOFPRINT_DATA")
        .arg("--data")
        .arg(dir)
        .args(args)
        .output()
        .expect("CLI starts")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn publishes_linked_records_with_artifact_and_proof() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("ledger");
    success(run(&dir, &["init"]));
    success(run(&dir, &["keygen"]));
    let artifact = temp.path().join("artifact.bin");
    std::fs::write(&artifact, [0, 255, 128, 1]).unwrap();
    let attachment = format!("checkpoint={}", artifact.display());
    let args = [
        "--json",
        "publish",
        "--kind",
        "example.note/v1",
        "--payload",
        r#"{"step":1}"#,
        "--created",
        "1700000000",
        "--blob",
        &attachment,
    ];
    let published: serde_json::Value = serde_json::from_str(&success(run(&dir, &args))).unwrap();
    let first = published["id"].as_str().unwrap();
    let repeat: serde_json::Value = serde_json::from_str(&success(run(&dir, &args))).unwrap();
    assert_eq!(repeat["position"], 0);
    let second = success(run(
        &dir,
        &["publish", "--kind", "example.note/v1", "--parent", first],
    ));
    let second = second.trim();
    let verified: serde_json::Value =
        serde_json::from_str(&success(run(&dir, &["--json", "verify", first]))).unwrap();
    assert_eq!(verified["blobs"]["verified_bytes"], 4);
    assert_eq!(verified["inclusion"]["tree_size"], 2);
    let proof: serde_json::Value =
        serde_json::from_str(&success(run(&dir, &["--json", "prove", second]))).unwrap();
    assert_eq!(proof["valid"], true);
    let tree: serde_json::Value =
        serde_json::from_str(&success(run(&dir, &["--json", "tree", second]))).unwrap();
    assert_eq!(tree["ancestors"][0]["id"], first);
    success(run(&dir, &["verify", "--all"]));

    let ledger = proofprint_core::Ledger::open(&dir).unwrap();
    let record = ledger.log.get(&first.parse().unwrap()).unwrap();
    let blob = record.record.blobs[0].blob.to_string();
    let blob_path = ledger.blobs.path_for(&record.record.blobs[0].blob);
    drop(ledger);
    let exported = run(&dir, &["blob", "get", &blob, "-"]);
    assert!(exported.status.success());
    assert_eq!(exported.stdout, [0, 255, 128, 1]);
    success(run(&dir, &["blob", "check", &blob]));
    let export_path = temp.path().join("export.bin");
    success(run(
        &dir,
        &[
            "--json",
            "blob",
            "get",
            &blob,
            export_path.to_str().unwrap(),
        ],
    ));
    assert_eq!(std::fs::read(export_path).unwrap(), [0, 255, 128, 1]);
    std::fs::write(blob_path, b"corrupt").unwrap();
    assert!(!run(&dir, &["--json", "verify", first]).status.success());
    assert!(!run(&dir, &["--json", "verify", "--all"]).status.success());
    assert!(!run(&dir, &["blob", "check", &blob]).status.success());
}

#[test]
fn warns_once_when_an_interrupted_write_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["keygen"]));
    success(run(dir.path(), &["publish", "--kind", "example.note/v1"]));
    let log = dir.path().join("log").join("records.ndjson");
    let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    std::io::Write::write_all(&mut file, br#"{"record":{"v":1,"#).unwrap();
    drop(file);

    let first = run(dir.path(), &["root"]);
    assert!(first.status.success());
    let warning = String::from_utf8_lossy(&first.stderr);
    assert!(warning.contains("dropped 17 bytes"), "{warning}");
    assert!(String::from_utf8_lossy(&first.stdout).contains("records 1"));

    let second = run(dir.path(), &["root"]);
    assert!(
        second.stderr.is_empty(),
        "the tail is gone after the first open"
    );
}

#[test]
fn rejects_invalid_timestamp_and_missing_parent_without_appending() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["keygen"]));
    let bad = run(
        dir.path(),
        &[
            "publish",
            "--kind",
            "example.note/v1",
            "--created",
            "9223372036854775807",
        ],
    );
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("timestamp out of range"));
    let parent = proofprint_core::RecordId::from_bytes(b"absent").to_string();
    assert!(!run(
        dir.path(),
        &["publish", "--kind", "example.note/v1", "--parent", &parent]
    )
    .status
    .success());
    assert!(proofprint_core::Ledger::open(dir.path())
        .unwrap()
        .log
        .is_empty());
}
