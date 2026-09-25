//! ProofPrint command-line interface.
//!
//! Everything here is a thin layer over [`proofprint_core`]. The CLI is a node
//! operator's tool: it manages keys, publishes records, and answers questions
//! about the local log. It is not privileged — any record can be re-verified
//! from the NDJSON log by any third party.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::args::{BlobCommand as BlobCmd, Cli, Command as Cmd, PublishArgs};
use anyhow::{bail, Context, Result};
use proofprint_core::key::KeyPair;
use proofprint_core::log::AppendOptions;
use proofprint_core::record::Record;
use proofprint_core::schema::KNOWN_KINDS;
use proofprint_core::{Ledger, RecordId};
use serde_json::{json, Value};

pub(crate) fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Cmd::Init => cmd_init(cli),
        Cmd::Keygen { force } => cmd_keygen(cli, *force),
        Cmd::Whoami => cmd_whoami(cli),
        Cmd::Publish(args) => cmd_publish(cli, args),
        Cmd::Get { id, payload_only } => cmd_get(cli, id, *payload_only),
        Cmd::Verify { id, all } => cmd_verify(cli, id.as_deref(), *all),
        Cmd::Log { limit, kind } => cmd_log(cli, *limit, kind.as_deref()),
        Cmd::Root => cmd_root(cli),
        Cmd::Prove { id } => cmd_prove(cli, id),
        Cmd::Tree { id } => cmd_tree(cli, id),
        Cmd::Blob { command } => cmd_blob(cli, command),
        Cmd::Schemas => cmd_schemas(cli),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_init(cli: &Cli) -> Result<()> {
    let ledger = open_ledger(cli)?;
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "data_dir": ledger.data_dir,
                "log_file": ledger.log.path(),
                "records": ledger.log.len(),
                "blobs": ledger.blobs.count()?,
            }))?
        );
    } else {
        println!("initialized {}", ledger.data_dir.display());
        println!("  log   {}", ledger.log.path().display());
        println!("  blobs {}", ledger.data_dir.join("blobs").display());
    }
    Ok(())
}

fn cmd_keygen(cli: &Cli, force: bool) -> Result<()> {
    let path = key_path(cli);
    if path.exists() && !force {
        bail!(
            "key already exists at {}\nuse --force to replace it; keep a backup to sign with the previous identity",
            path.display()
        );
    }
    let keys = KeyPair::generate();
    if force {
        keys.replace(&path)?;
    } else {
        keys.save(&path)?;
    }
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "key": path, "signer": keys.signer() }))?
        );
    } else {
        println!("wrote {}", path.display());
        println!("signer {}", keys.signer());
    }
    Ok(())
}

fn cmd_whoami(cli: &Cli) -> Result<()> {
    let keys = load_key(cli)?;
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "signer": keys.signer() }))?
        );
    } else {
        println!("{}", keys.signer());
    }
    Ok(())
}

fn cmd_publish(cli: &Cli, args: &PublishArgs) -> Result<()> {
    let keys = load_key(cli)?;
    let mut ledger = open_ledger(cli)?;

    let payload = read_payload(&args.payload)?;
    if !payload.is_object() {
        bail!("payload must be a JSON object");
    }

    let mut refs = Vec::with_capacity(args.blobs.len());
    for spec in &args.blobs {
        let (name, path) = parse_blob_spec(spec)?;
        let r = ledger
            .attach_file(name, &path, None)
            .with_context(|| format!("attaching blob from {}", path.display()))?;
        refs.push(r);
    }

    let mut parent_ids = Vec::with_capacity(args.parents.len());
    for p in &args.parents {
        parent_ids.push(RecordId::parse(p).with_context(|| format!("invalid --parent {p:?}"))?);
    }

    let mut record = Record::new(&args.kind, payload, keys.signer()).with_parents(parent_ids);
    for r in refs {
        record = record.with_blob(r);
    }
    if let Some(secs) = args.created {
        record = record.with_created(
            chrono::DateTime::from_timestamp(secs, 0).context("timestamp out of range")?,
        );
    }

    let signed = record.sign(&keys)?;
    let id = signed.id()?;
    ledger.append_with(
        signed.clone(),
        AppendOptions {
            require_parents_present: !args.allow_missing_parents,
        },
    )?;
    let position = ledger
        .log
        .position_of(&id)
        .context("appended record missing")?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "id": id.to_string(),
                "position": position,
                "root": ledger.log.root_hex(),
                "record": signed,
            }))?
        );
    } else {
        println!("{id}");
    }
    Ok(())
}

fn cmd_get(cli: &Cli, id_str: &str, payload_only: bool) -> Result<()> {
    let id = RecordId::parse(id_str)?;
    let ledger = open_ledger(cli)?;
    let Some(signed) = ledger.log.get(&id) else {
        bail!("record not found in this log: {id}");
    };

    let value = if payload_only {
        signed.record.payload.clone()
    } else {
        serde_json::to_value(signed)?
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn cmd_verify(cli: &Cli, id_str: Option<&str>, all: bool) -> Result<()> {
    let ledger = open_ledger(cli)?;

    if all {
        let total = ledger.log.len();
        let mut failures = Vec::new();
        for (pos, id, signed) in ledger.log.iter() {
            if let Err(e) = signed.verify() {
                failures
                    .push(json!({"position": pos, "id": id.to_string(), "error": e.to_string()}));
                continue;
            }
            let audit = ledger.verify_record_blobs(&signed.record)?;
            if !audit.is_consistent() {
                failures.push(json!({
                    "position": pos,
                    "id": id.to_string(),
                    "error": "blob mismatch",
                    "corrupt": audit.corrupt,
                }));
            }
        }
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "records": total,
                    "failures": failures,
                    "root": ledger.log.root_hex(),
                }))?
            );
        } else if failures.is_empty() {
            println!("ok: {total} records verified");
            if let Some(root) = ledger.log.root_hex() {
                println!("root: {root}");
            }
        } else {
            println!("FAILED: {} of {total} records", failures.len());
            for f in &failures {
                println!("  {f}");
            }
        }
        if !failures.is_empty() {
            bail!("verification failed");
        }
        return Ok(());
    }

    let Some(id_str) = id_str else {
        bail!("provide a record id, or --all");
    };
    let id = RecordId::parse(id_str)?;
    let Some(signed) = ledger.log.get(&id) else {
        bail!("record not found in this log: {id}");
    };

    let verified_id = signed.verify().context("signature verification failed")?;
    let audit = ledger.verify_record_blobs(&signed.record)?;
    let proof = ledger.log.inclusion_proof(&id)?;
    proof.verify().context("inclusion proof failed")?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "id": verified_id.to_string(),
                "kind": signed.record.kind,
                "signer": signed.record.signer,
                "created": signed.record.created.to_rfc3339(),
                "signature": "valid",
                "inclusion": {
                    "leaf_index": proof.leaf_index,
                    "tree_size": proof.tree_size,
                    "root": hex::encode(proof.root),
                    "path_len": proof.path.len(),
                    "valid": true,
                },
                "blobs": {
                    "verified": audit.verified,
                    "missing": audit.missing,
                    "corrupt": audit.corrupt,
                    "verified_bytes": audit.verified_bytes,
                },
            }))?
        );
    } else {
        println!("record      {verified_id}");
        println!("kind        {}", signed.record.kind);
        println!("signer      {}", signed.record.signer);
        println!("created     {}", signed.record.created.to_rfc3339());
        println!("signature   valid");
        println!(
            "inclusion   index {} of {} (path len {})",
            proof.leaf_index,
            proof.tree_size,
            proof.path.len()
        );
        println!("root        {}", hex::encode(proof.root));
        println!(
            "blobs       {} verified, {} missing, {} corrupt",
            audit.verified.len(),
            audit.missing.len(),
            audit.corrupt.len()
        );
    }
    if !audit.is_consistent() {
        bail!("blob verification failed");
    }
    Ok(())
}

fn cmd_log(cli: &Cli, limit: usize, kind: Option<&str>) -> Result<()> {
    let ledger = open_ledger(cli)?;
    let rows: Vec<(usize, String, String, String, String)> = ledger
        .log
        .iter()
        .filter(|(_, _, r)| kind.is_none_or(|k| r.record.kind == k))
        .map(|(pos, id, r)| {
            (
                pos,
                id.to_string(),
                r.record.kind.clone(),
                r.record.created.to_rfc3339(),
                r.record.signer.clone(),
            )
        })
        .collect();

    let shown: Vec<_> = if limit == 0 {
        rows.clone()
    } else {
        rows.iter().rev().take(limit).cloned().rev().collect()
    };

    if cli.json {
        let items: Vec<Value> = shown
            .iter()
            .map(|(pos, id, kind, created, signer)| {
                json!({"position": pos, "id": id, "kind": kind, "created": created, "signer": signer})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "count": rows.len(), "shown": items }))?
        );
    } else {
        println!(
            "{} record(s) in {}",
            rows.len(),
            ledger.log.path().display()
        );
        for (pos, id, kind, created, signer) in &shown {
            println!(
                "#{pos:<6} {}  {kind:<28} {created}  {}",
                short_id(id),
                short_signer(signer)
            );
        }
    }
    Ok(())
}

fn cmd_root(cli: &Cli) -> Result<()> {
    let ledger = open_ledger(cli)?;
    let root = ledger.log.root_hex();
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "records": ledger.log.len(),
                "root": root,
            }))?
        );
    } else {
        println!("records {}", ledger.log.len());
        println!("root    {}", root.unwrap_or_else(|| "(empty log)".into()));
    }
    Ok(())
}

fn cmd_prove(cli: &Cli, id_str: &str) -> Result<()> {
    let id = RecordId::parse(id_str)?;
    let ledger = open_ledger(cli)?;
    let proof = ledger
        .log
        .inclusion_proof(&id)
        .with_context(|| format!("{id} is not in this log"))?;
    proof.verify()?;

    let value = json!({
        "record": id.to_string(),
        "leaf_index": proof.leaf_index,
        "tree_size": proof.tree_size,
        "leaf_hash": hex::encode(proof.leaf_hash),
        "path": proof.path.iter().map(hex::encode).collect::<Vec<_>>(),
        "root": hex::encode(proof.root),
        "valid": true,
    });

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("record    {id}");
        println!("index     {} of {}", proof.leaf_index, proof.tree_size);
        println!("leaf      {}", hex::encode(proof.leaf_hash));
        println!("path      {} node(s)", proof.path.len());
        for (i, h) in proof.path.iter().enumerate() {
            println!("  [{i}] {}", hex::encode(h));
        }
        println!("root      {}", hex::encode(proof.root));
    }
    Ok(())
}

fn cmd_tree(cli: &Cli, id_str: &str) -> Result<()> {
    let id = RecordId::parse(id_str)?;
    let ledger = open_ledger(cli)?;
    let Some(signed) = ledger.log.get(&id) else {
        bail!("record not found in this log: {id}");
    };

    let ancestors = ledger.log.ancestors_of(&id);
    let descendants = ledger.log.descendants_of(&id);

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "record": describe(&ledger, &id, signed),
                "ancestors": ancestors.iter().map(|a| describe_by_id(&ledger, a)).collect::<Vec<_>>(),
                "descendants": descendants.iter().map(|d| describe_by_id(&ledger, d)).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    println!("ancestors ({}):", ancestors.len());
    if ancestors.is_empty() {
        println!("  (none — this is a root record)");
    }
    for a in &ancestors {
        print_line(&ledger, a, "  ");
    }

    println!("\nthis record:");
    print_line(&ledger, &id, "  ");

    println!("\ndescendants ({}):", descendants.len());
    if descendants.is_empty() {
        println!("  (none — this is a leaf)");
    }
    for d in &descendants {
        print_line(&ledger, d, "  ");
    }
    Ok(())
}

fn cmd_blob(cli: &Cli, command: &BlobCmd) -> Result<()> {
    match command {
        BlobCmd::Add { path, name, media } => {
            let ledger = open_ledger(cli)?;
            let name = name.clone().unwrap_or_else(|| {
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "artifact".into())
            });
            let r = ledger.attach_file(name, path, media.clone())?;
            let value = serde_json::to_value(&r)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("{}  {} bytes  {}", r.blob, r.size, r.name);
            }
        }
        BlobCmd::Get { id, out } => {
            use proofprint_core::BlobId;
            let blob = BlobId::parse(id)?;
            let ledger = open_ledger(cli)?;
            let mut reader = ledger.blobs.open_reader(&blob)?;
            if out == "-" {
                std::io::copy(&mut reader, &mut std::io::stdout().lock())?;
            } else {
                let dest = Path::new(out);
                if let Some(parent) = dest.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)?;
                    }
                }
                let parent = dest
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let mut staging = tempfile::NamedTempFile::new_in(parent)?;
                let size = std::io::copy(&mut reader, &mut staging)?;
                staging.as_file().sync_all()?;
                staging.persist(dest)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({"path": dest, "size": size}))?
                    );
                } else {
                    println!("wrote {} bytes to {}", size, dest.display());
                }
            }
        }
        BlobCmd::Check { id } => {
            use proofprint_core::BlobId;
            let blob = BlobId::parse(id)?;
            let ledger = open_ledger(cli)?;
            let size = ledger.blobs.verify(&blob)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "blob": id, "size": size, "consistent": true
                    }))?
                );
            } else {
                println!("{}  {} bytes  consistent", id, size);
            }
        }
        BlobCmd::Stats => {
            let ledger = open_ledger(cli)?;
            let n = ledger.blobs.count()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&json!({ "blobs": n }))?);
            } else {
                println!("{n} blob(s) in {}", ledger.data_dir.join("blobs").display());
            }
        }
    }
    Ok(())
}

fn cmd_schemas(cli: &Cli) -> Result<()> {
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "kinds": KNOWN_KINDS }))?
        );
    } else {
        println!("record kinds known to this build:");
        for k in KNOWN_KINDS {
            println!("  {k}");
        }
        println!("\nAny `<name>/v<N>` kind is accepted; these are the ones with typed schemas.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open the ledger, and say so on stderr if an interrupted write was dropped.
fn open_ledger(cli: &Cli) -> Result<Ledger> {
    let ledger = Ledger::open(&cli.data)
        .with_context(|| format!("opening ledger in {}", cli.data.display()))?;
    let dropped = ledger.log.recovered_bytes();
    if dropped > 0 {
        eprintln!(
            "warning: dropped {dropped} bytes of an interrupted write from the end of {}",
            ledger.log.path().display()
        );
    }
    Ok(ledger)
}

fn key_path(cli: &Cli) -> PathBuf {
    cli.key
        .clone()
        .unwrap_or_else(|| cli.data.join("keys").join("default.key"))
}

fn load_key(cli: &Cli) -> Result<KeyPair> {
    let path = key_path(cli);
    KeyPair::load(&path).with_context(|| format!("loading signing key {}", path.display()))
}

/// Read a payload from inline JSON, `@file`, or `-` (stdin).
fn read_payload(spec: &str) -> Result<Value> {
    let text = if spec == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading payload from stdin")?;
        buf
    } else if let Some(path) = spec.strip_prefix('@') {
        fs::read_to_string(path).with_context(|| format!("reading payload from {path}"))?
    } else {
        spec.to_string()
    };

    serde_json::from_str(&text).context("payload is not valid JSON")
}

fn parse_blob_spec(spec: &str) -> Result<(String, PathBuf)> {
    let Some((name, path)) = spec.split_once('=') else {
        bail!("--blob expects name=path, got {spec:?}");
    };
    if name.is_empty() {
        bail!("--blob name cannot be empty: {spec:?}");
    }
    if path.is_empty() {
        bail!("--blob path cannot be empty: {spec:?}");
    }
    Ok((name.to_string(), PathBuf::from(path)))
}

fn short_id(id: &str) -> String {
    // "ppr1:" + first 12 hex chars + ellipsis
    let body = id.strip_prefix("ppr1:").unwrap_or(id);
    let head: String = body.chars().take(12).collect();
    format!("ppr1:{head}...")
}

fn short_signer(signer: &str) -> String {
    let body = signer.strip_prefix("ed25519:").unwrap_or(signer);
    let head: String = body.chars().take(12).collect();
    format!("ed25519:{head}...")
}

fn describe(ledger: &Ledger, id: &RecordId, signed: &proofprint_core::SignedRecord) -> Value {
    let availability: Value = ledger
        .blob_availability(&signed.record)
        .into_iter()
        .map(|(name, present)| json!({"name": name, "present": present}))
        .collect();
    json!({
        "id": id.to_string(),
        "position": ledger.log.position_of(id),
        "kind": signed.record.kind,
        "signer": signed.record.signer,
        "created": signed.record.created.to_rfc3339(),
        "parents": signed.record.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        "blobs": availability,
    })
}

fn describe_by_id(ledger: &Ledger, id: &RecordId) -> Value {
    match ledger.log.get(id) {
        Some(signed) => describe(ledger, id, signed),
        None => json!({"id": id.to_string(), "present": false}),
    }
}

fn print_line(ledger: &Ledger, id: &RecordId, indent: &str) {
    match ledger.log.get(id) {
        Some(signed) => println!(
            "{indent}{}  {:<28} {}",
            short_id(&id.to_string()),
            signed.record.kind,
            signed.record.created.to_rfc3339()
        ),
        None => println!("{indent}{}  (not in this log)", short_id(&id.to_string())),
    }
}
