//! `proofprint-node`: serve one local ledger over HTTP.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use proofprint_core::{Error, KeyPair, Ledger};
use proofprint_node::{demo, router, Node, Settings};
use tracing_subscriber::EnvFilter;

/// Serve the ProofPrint API and, when built, the explorer.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Directory holding blobs and the append-only log.
    #[arg(long, default_value = ".proofprint", env = "PROOFPRINT_DATA")]
    data: PathBuf,
    /// Key used to sign drafts sent to /api/publish. Defaults to
    /// <data>/keys/default.key, which is created on first start.
    #[arg(long, env = "PROOFPRINT_KEY")]
    key: Option<PathBuf>,
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:4780")]
    listen: SocketAddr,
    /// Accept a non-loopback listen address. The API has no authentication.
    #[arg(long)]
    allow_remote: bool,
    /// Directory produced by building the explorer.
    #[arg(long, default_value = "explorer/dist")]
    ui: PathBuf,
    /// Largest accepted artifact upload, in MiB.
    #[arg(long, default_value_t = 4096)]
    max_upload_mib: u64,
    /// Fill an empty ledger with labeled demonstration records.
    #[arg(long)]
    demo: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "proofprint_node=info,tower_http=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let args = Args::parse();
    anyhow::ensure!(
        args.listen.ip().is_loopback() || args.allow_remote,
        "{} is not a loopback address; pass --allow-remote to expose the unauthenticated API",
        args.listen
    );

    let mut ledger = Ledger::open(&args.data)
        .with_context(|| format!("opening ledger in {}", args.data.display()))?;
    if ledger.log.recovered_bytes() > 0 {
        tracing::warn!(
            bytes = ledger.log.recovered_bytes(),
            "dropped an interrupted write from the end of the log"
        );
    }
    let keys = load_keys(&args)?;
    if args.demo {
        let ids = demo::seed(&mut ledger, &keys)?;
        if !ids.is_empty() {
            tracing::info!(records = ids.len(), "seeded demonstration records");
        }
    }

    let node = Arc::new(Node::new(ledger, Some(keys)));
    let settings = Settings {
        ui: args.ui.clone(),
        upload_limit: args.max_upload_mib.saturating_mul(1 << 20),
        loopback_only: args.listen.ip().is_loopback(),
    };
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(
        address = %listener.local_addr()?,
        data = %args.data.display(),
        signer = node.signer().unwrap_or_default(),
        "node ready"
    );
    if !args.ui.join("index.html").is_file() {
        tracing::info!(
            ui = %args.ui.display(),
            "no explorer build found; run `npm run build` in explorer/ to serve it"
        );
    }
    axum::serve(listener, router(node, settings))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Load the signing key. A missing key at the default path is created; a
/// missing key at an explicit `--key` path is an error.
fn load_keys(args: &Args) -> anyhow::Result<KeyPair> {
    let default_path = args.data.join("keys").join("default.key");
    let path = args.key.clone().unwrap_or_else(|| default_path.clone());
    match KeyPair::load(&path) {
        Ok(keys) => Ok(keys),
        Err(Error::KeyNotFound(_)) if args.key.is_none() => {
            let keys = KeyPair::generate();
            keys.save(&path)?;
            tracing::warn!(path = %path.display(), "created a new signing key");
            Ok(keys)
        }
        Err(error) => Err(error).with_context(|| format!("loading signing key {}", path.display())),
    }
}
