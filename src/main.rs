mod hash_v1;
mod hash_v2;
mod http;
mod magnet;
mod metainfo;
mod trackers;
mod util;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use data_encoding::BASE32_NOPAD;
use futures::StreamExt;
use hash_v1::V1Hasher;
use hash_v2::V2Hasher;
use magnet::build_magnets;
use metainfo::{build as build_metainfo, BuildInput};
use reqwest::Client;
use tokio::time::Instant;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use url::Url;

use crate::util::{choose_piece_length, format_bytes, sanitize_filename};

#[derive(Debug, Parser)]
#[command(name = "torseed", version, about = "Create hybrid BitTorrent torrents from HTTP sources")]
struct Cli {
    /// Primary HTTP/HTTPS URL to fetch and hash
    #[arg(value_name = "URL")]
    primary_url: String,

    /// Additional HTTP(S) URLs to include as webseeds
    #[arg(value_name = "WEBSEED", num_args = 0..)]
    extra_urls: Vec<String>,

    /// Optional output path for the torrent file
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let client = build_client()?;

    let primary_url = parse_url(&cli.primary_url)?;
    info!("Primary URL: {}", primary_url);

    let primary_meta = http::head_source(&client, primary_url.clone())
        .await
        .with_context(|| format!("Failed to fetch metadata for {primary_url}"))?;

    let mut webseeds: Vec<String> = Vec::new();
    webseeds.push(primary_meta.url.to_string());

    let mut extra_urls: Vec<Url> = Vec::new();
    for value in cli.extra_urls {
        let url = parse_url(&value)?;
        extra_urls.push(url);
    }

    let extra_webseeds = verify_webseeds(&client, primary_meta.content_length, extra_urls).await;
    for url in extra_webseeds {
        webseeds.push(url.to_string());
    }

    let trackers = trackers::gather_trackers(&client)
        .await
        .context("Failed to gather tracker list")?;

    let piece_length = choose_piece_length(primary_meta.content_length);
    info!(
        "Using v1 piece length {} KiB ({} pieces)",
        piece_length / 1024,
        (primary_meta.content_length + piece_length as u64 - 1) / piece_length as u64
    );

    let mut v1_hasher = V1Hasher::new(piece_length);
    let mut v2_hasher = V2Hasher::new().context("Failed to initialize v2 hasher")?;
    let mut total_bytes: u64 = 0;

    let response = http::stream(&client, &primary_meta.url)
        .await
        .with_context(|| format!("Failed to stream data from {}", primary_meta.url))?;

    let mut stream = response.bytes_stream();
    let mut last_log = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| "Error while reading HTTP stream")?;
        total_bytes += chunk.len() as u64;
        v1_hasher.update(&chunk);
        v2_hasher
            .update(&chunk)
            .context("Failed while hashing for v2")?;

        if last_log.elapsed() > Duration::from_secs(15) {
            let pct = (total_bytes as f64 / primary_meta.content_length as f64) * 100.0;
            info!("Hashed {:.1}% ({} / {})", pct, format_bytes(total_bytes), format_bytes(primary_meta.content_length));
            last_log = Instant::now();
        }
    }

    if total_bytes != primary_meta.content_length {
        warn!(
            "Streamed size mismatch: expected {} bytes, got {} bytes",
            primary_meta.content_length,
            total_bytes
        );
    }

    let pieces = v1_hasher.finalize();
    let v2_summary = match v2_hasher.finalize(piece_length) {
        Ok(summary) => Some(summary),
        Err(err) => {
            warn!("Falling back to v1-only torrent: {err}");
            None
        }
    };

    let creation_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let output_path = compute_output_path(cli.output, &primary_meta.filename);
    let created_by = format!("torseed {}", env!("CARGO_PKG_VERSION"));

    let build_input = BuildInput {
        name: sanitize_filename(&primary_meta.filename),
        length: primary_meta.content_length,
        piece_length: u32::try_from(piece_length).context("piece length overflow")?,
        pieces,
        trackers: trackers.clone(),
        webseeds: webseeds.clone(),
        creation_date,
        created_by,
        v2: v2_summary,
    };

    let metainfo = build_metainfo(&build_input)?;

    write_torrent(&output_path, &metainfo.torrent)?;

    let magnets = build_magnets(
        &build_input.name,
        &trackers,
        &webseeds,
        metainfo.infohash_v1,
        metainfo.infohash_v2,
    );

    let magnet_path = magnet_output_path(&output_path);
    write_magnet_file(&magnet_path, &magnets)?;

    print_summary(
        &output_path,
        &build_input,
        &metainfo,
        &trackers,
        &webseeds,
        &magnets,
        &magnet_path,
    );

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("torseed/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("Failed to build HTTP client")
}

fn parse_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).with_context(|| format!("Invalid URL: {input}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => anyhow::bail!("Unsupported URL scheme: {other}"),
    }
}

async fn verify_webseeds(client: &Client, expected_length: u64, urls: Vec<Url>) -> Vec<Url> {
    use futures::stream::FuturesUnordered;

    let mut verified = Vec::new();
    let mut tasks = FuturesUnordered::new();
    for url in urls {
        let client = client.clone();
        tasks.push(async move {
            match http::head_source(&client, url.clone()).await {
                Ok(meta) => Some((url, meta)),
                Err(err) => {
                    warn!("Skipping webseed {url}: {err}");
                    None
                }
            }
        });
    }

    let mut now = Instant::now();
    while let Some(result) = tasks.next().await {
        if let Some((url, meta)) = result {
            if meta.content_length == expected_length {
                verified.push(url);
            } else {
                warn!(
                    "Skipping webseed {} (length mismatch: {} vs {expected_length})",
                    url,
                    meta.content_length
                );
            }
        }
        if now.elapsed() > Duration::from_secs(10) {
            info!("Checked {} webseeds", verified.len());
            now = Instant::now();
        }
    }

    verified
}

fn compute_output_path(cli_value: Option<PathBuf>, filename: &str) -> PathBuf {
    if let Some(path) = cli_value {
        return path;
    }
    let sanitized = sanitize_filename(filename);
    PathBuf::from(format!("{sanitized}.torrent"))
}

fn write_torrent(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directories for {}", path.display()))?;
        }
    }
    std::fs::write(path, bytes)
        .with_context(|| format!("Failed to write torrent file to {}", path.display()))
}

fn print_summary(
    output_path: &PathBuf,
    build_input: &BuildInput,
    metainfo: &metainfo::Metainfo,
    trackers: &[String],
    webseeds: &[String],
    magnets: &[String],
    magnet_path: &Path,
) {
    println!("Torrent written to {}", output_path.display());

    if let Some(v1) = metainfo.infohash_v1 {
        println!("v1 infohash (hex): {}", hex::encode(v1));
        println!("v1 infohash (base32): {}", BASE32_NOPAD.encode(&v1));
    }
    if let Some(v2) = metainfo.infohash_v2 {
        println!("v2 infohash (sha256 hex): {}", hex::encode(v2));
    }

    for magnet_uri in magnets {
        println!("magnet: {}", magnet_uri);
    }
    println!("Magnet links written to {}", magnet_path.display());

    let pieces = build_input.pieces.len() / 20;
    println!(
        "File size: {} ({} bytes)",
        format_bytes(build_input.length),
        build_input.length
    );
    println!(
        "Piece length: {} KiB",
        build_input.piece_length / 1024
    );
    println!("Pieces: {}", pieces);
    println!("Trackers: {}", trackers.len());
    println!("Webseeds: {}", webseeds.len());
}

fn write_magnet_file(path: &Path, magnets: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directories for {}", path.display()))?;
        }
    }

    let mut contents = magnets.join("\n");
    contents.push('\n');
    fs::write(path, contents).with_context(|| format!("Failed to write magnet file to {}", path.display()))
}

fn magnet_output_path(output_path: &Path) -> PathBuf {
    let dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    dir.join(".magnet")
}
