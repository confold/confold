//! Local, no-Docker S3-compatible server for testing Confold's S3 source by hand.
//!
//! Runs `s3s` + `s3s-fs` (pure Rust) over a host directory on a fixed localhost port, so you can point
//! a Confold S3 source at it without MinIO or Docker. The served directory holds one bucket; the files
//! you drop under `<dir>/<bucket>/` are what Confold compares/migrates/syncs against.
//!
//! Usage:  cargo run --example s3-demo -p confold-s3 -- [dir] [port]   (or scripts/s3-demo.sh)
//! Stop with Ctrl-C.

use std::path::PathBuf;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use tokio::net::TcpListener;

const ACCESS_KEY: &str = "confold";
const SECRET_KEY: &str = "confold-secret";
const BUCKET: &str = "data";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| "/tmp/confold-s3".to_string()));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(4566);

    // s3s-fs maps a bucket to a subdirectory of the served root; create it up front.
    let bucket_dir = dir.join(BUCKET);
    std::fs::create_dir_all(&bucket_dir)?;
    // Seed a small tree the first time, so there's something to compare against.
    let seed = bucket_dir.join("readme.txt");
    if !seed.exists() {
        std::fs::write(&seed, b"hello from the Confold S3 demo\n")?;
        std::fs::create_dir_all(bucket_dir.join("sub"))?;
        std::fs::write(bucket_dir.join("sub").join("note.txt"), b"nested file\n")?;
    }

    let fs = FileSystem::new(&dir).map_err(|e| anyhow::anyhow!("s3s-fs init failed: {e:?}"))?;
    let service = {
        let mut b = S3ServiceBuilder::new(fs);
        b.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
        b.build()
    };

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;

    println!("──────────────────────────────────────────────────────────────");
    println!(" Confold S3 demo server (pure Rust — no Docker)");
    println!("   Endpoint  : http://{addr}");
    println!("   Region    : us-east-1");
    println!("   Bucket    : {BUCKET}");
    println!("   Access key: {ACCESS_KEY}");
    println!("   Secret key: {SECRET_KEY}");
    println!("   Serves    : {}  →  bucket '{BUCKET}'", dir.display());
    println!();
    println!("   Paste URL : s3://{ACCESS_KEY}:{SECRET_KEY}@{addr}/{BUCKET}");
    println!("   (paste it into a Confold source picker's URL field, or fill the fields above; Ctrl-C to stop.)");
    println!("──────────────────────────────────────────────────────────────");

    let http = ConnBuilder::new(TokioExecutor::new());
    loop {
        let (socket, _) = listener.accept().await?;
        let svc = service.clone();
        let http = http.clone();
        tokio::spawn(async move {
            let _ = http.serve_connection(TokioIo::new(socket), svc).await;
        });
    }
}
