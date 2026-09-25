//! Command-line syntax. Execution lives in `commands`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// ProofPrint — a verifiable audit ledger for arbitrary artifacts.
#[derive(Parser, Debug)]
#[command(name = "proofprint", version, about)]
pub(crate) struct Cli {
    /// Directory holding blobs and the append-only log.
    #[arg(
        long,
        global = true,
        env = "PROOFPRINT_DATA",
        default_value = ".proofprint",
        value_name = "DIR"
    )]
    pub data: PathBuf,
    /// Signing key. Defaults to <data>/keys/default.key.
    #[arg(long, global = true, env = "PROOFPRINT_KEY", value_name = "PATH")]
    pub key: Option<PathBuf>,
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Create the local storage layout.
    Init,
    /// Generate a signing key.
    Keygen {
        /// Replace an existing key file.
        #[arg(long)]
        force: bool,
    },
    /// Print the public signer identifier.
    Whoami,
    /// Sign and append a record.
    Publish(PublishArgs),
    /// Read a record.
    Get {
        id: String,
        #[arg(long)]
        payload_only: bool,
    },
    /// Verify signatures and locally available attachments.
    Verify {
        #[arg(required_unless_present = "all")]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        all: bool,
    },
    /// List records in log order.
    Log {
        /// Maximum entries; zero shows all entries.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Print the current root and tree size.
    Root,
    /// Produce an inclusion proof.
    Prove { id: String },
    /// Inspect ancestors and descendants.
    Tree { id: String },
    /// Manage content-addressed artifacts.
    Blob {
        #[command(subcommand)]
        command: BlobCommand,
    },
    /// List bundled example schemas.
    Schemas,
}

#[derive(Args, Debug)]
pub(crate) struct PublishArgs {
    /// Schema reference, such as ml.training-step/v1.
    #[arg(long)]
    pub kind: String,
    /// Inline JSON, @file.json, or - for stdin.
    #[arg(long, default_value = "{}")]
    pub payload: String,
    /// Parent record ID; repeat for multiple parents.
    #[arg(long = "parent", value_name = "ID")]
    pub parents: Vec<String>,
    /// Attachment; repeat for multiple files.
    #[arg(long = "blob", value_name = "NAME=PATH")]
    pub blobs: Vec<String>,
    /// Publisher timestamp in Unix seconds; defaults to now.
    #[arg(long, value_name = "SECONDS")]
    pub created: Option<i64>,
    /// Permit references to parents unavailable in this log.
    #[arg(long)]
    pub allow_missing_parents: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BlobCommand {
    /// Import a file.
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        media: Option<String>,
    },
    /// Export a blob; use - for binary stdout.
    Get { id: String, out: String },
    /// Check a stored artifact against its content ID.
    Check { id: String },
    /// Count stored artifacts.
    Stats,
}
