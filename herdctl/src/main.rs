//! `herdctl` — iroh-bound control client for `herd-scout-daemon`.
//!
//! Wave 11 / B2: a small fast-compiling client binary that opens a QUIC
//! bi-stream to a daemon on the [`herd_scout_ipc::CONTROL_ALPN`] protocol
//! and pipes stdin/stdout. Designed for use as an OpenSSH `ProxyCommand`:
//!
//! ```text
//! Host pasture
//!   ProxyCommand herdctl proxy <node-id>
//!   User pi
//! ```
//!
//! The local identity is persisted at
//! `$XDG_CONFIG_HOME/herdctl/identity.toml` via [`herd_scout_identity`].
//! Wave 12 Phase 0 moved the on-disk format from raw 32 bytes to a
//! versioned TOML envelope; legacy `secret.key` files are auto-upgraded
//! the first time `herdctl` runs after the upgrade.

mod upload;

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::presets};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "herdctl", version, about = "iroh-bound control client for herd-scout-daemon")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Pipe stdin/stdout to a herd-scout-daemon over iroh. Use as OpenSSH ProxyCommand.
    Proxy { node_id: String },
    /// Connect-and-close health check; exit 0 if the daemon's allowlist accepts us.
    Ping { node_id: String },
    /// Print the local NodeId; paste into ~/.ssh/config or daemon's control.toml.
    Whoami,
    /// Upload a video clip to a daemon for CV processing.
    Push {
        /// Daemon NodeId (canonical EndpointId string).
        node_id: String,
        /// Local path to the MP4/MOV/M4V clip.
        path: PathBuf,
        /// Skip waiting for processing to finish; exit after acceptance.
        #[arg(long)]
        no_wait: bool,
    },
    /// Manage queued / recent uploads on a daemon.
    Uploads {
        #[command(subcommand)]
        op: UploadsOp,
    },
}

#[derive(Subcommand, Debug)]
enum UploadsOp {
    /// List the daemon's upload queue.
    List { node_id: String },
    /// Cancel a queued upload by full BLAKE3 hex (or unique prefix).
    Cancel {
        node_id: String,
        blake3_prefix: String,
    },
    /// Fetch the JSON report for a finished clip.
    Report {
        node_id: String,
        blake3_prefix: String,
        /// Print pretty-formatted JSON instead of headline summary.
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,herdctl=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Proxy { node_id } => proxy(&node_id).await,
        Cmd::Ping { node_id } => ping(&node_id).await,
        Cmd::Whoami => whoami().await,
        Cmd::Push {
            node_id,
            path,
            no_wait,
        } => upload::push(&node_id, &path, no_wait).await,
        Cmd::Uploads { op } => match op {
            UploadsOp::List { node_id } => upload::list(&node_id).await,
            UploadsOp::Cancel {
                node_id,
                blake3_prefix,
            } => upload::cancel(&node_id, &blake3_prefix).await,
            UploadsOp::Report {
                node_id,
                blake3_prefix,
                json,
            } => upload::report(&node_id, &blake3_prefix, json).await,
        },
    }
}

/// Path to the persisted herdctl identity envelope.
fn identity_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herdctl")
        .context("could not resolve project dirs")?;
    Ok(dirs.config_dir().join("identity.toml"))
}

pub(crate) async fn make_endpoint() -> Result<Endpoint> {
    let path = identity_path()?;
    let id = herd_scout_identity::load_or_generate(&path, "herdctl")
        .with_context(|| format!("load or create identity at {}", path.display()))?;
    Endpoint::builder(presets::N0)
        .secret_key(id.secret)
        .bind()
        .await
        .context("bind iroh endpoint")
}

async fn proxy(node_id: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), herd_scout_ipc::CONTROL_ALPN)
            .await
            .context("dial daemon")?;
        let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;

        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();

        let to_remote = async {
            tokio::io::copy(&mut stdin, &mut send).await?;
            // Half-close: tells the daemon we're done sending. Ignored
            // if the stream was already closed by the peer.
            let _ = send.finish();
            anyhow::Ok(())
        };
        let from_remote = async {
            tokio::io::copy(&mut recv, &mut stdout).await?;
            stdout.flush().await?;
            anyhow::Ok(())
        };

        // We don't bail on either half ending — that's a normal session
        // shutdown (one side closes its half-stream).
        let _ = tokio::try_join!(to_remote, from_remote);
        anyhow::Ok(())
    }
    .await;
    ep.close().await;
    result
}

async fn ping(node_id: &str) -> Result<()> {
    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), herd_scout_ipc::CONTROL_ALPN)
            .await
            .context("dial daemon")?;
        // QUIC's `open_bi()` succeeds purely on local stream allocation; the
        // server may still drop us via the allowlist gate before
        // `accept_bi()`-ing. The real proof of authorization is that the
        // daemon then byte-bridges us to sshd, which sends an `SSH-2.0-…`
        // banner within milliseconds. If we were rejected, `recv.read` will
        // see a closed stream / connection-closed error.
        let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;
        // Push a few bytes upstream first. The daemon's `tokio::io::copy`
        // on the QUIC SendStream flushes incrementally only when there's
        // pressure on the read half — without our nudge, sshd's banner
        // sits in the daemon's QUIC send buffer until idle flush. Sending
        // a stub SSH client-banner-prefix here triggers the bidi flow.
        send.write_all(b"SSH-2.0-herdctl-ping\r\n").await
            .context("write ping nudge — daemon may have dropped us silently")?;
        let mut buf = [0u8; 4];
        tokio::time::timeout(std::time::Duration::from_secs(3), recv.read_exact(&mut buf))
            .await
            .context("ping timed out before reading sshd banner — daemon may have dropped us silently")?
            .context("read from daemon — likely allowlist drop or sshd not running")?;
        if &buf[..] != b"SSH-" {
            anyhow::bail!("first 4 bytes from daemon were not 'SSH-': {:?}", buf);
        }
        anyhow::Ok(())
    }
    .await;
    ep.close().await;
    result?;
    println!("ok");
    Ok(())
}

async fn whoami() -> Result<()> {
    let ep = make_endpoint().await?;
    println!("{}", ep.id());
    ep.close().await;
    Ok(())
}
