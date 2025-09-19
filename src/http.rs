use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{header, Client, Response, StatusCode};
use tracing::debug;
use url::Url;

use crate::util::sanitize_filename;

#[derive(Debug, Clone)]
pub struct SourceMetadata {
    pub url: Url,
    pub content_length: u64,
    pub filename: String,
}

pub async fn head_source(client: &Client, url: Url) -> Result<SourceMetadata> {
    let response = client
        .head(url.as_str())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .with_context(|| format!("HEAD request failed for {url}"))?;

    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        return fetch_via_get(client, url).await;
    }

    let status = response.status();
    let response = response
        .error_for_status()
        .with_context(|| format!("HEAD request returned error status {} for {url}", status))?;

    build_metadata(url, &response)
}

async fn fetch_via_get(client: &Client, url: Url) -> Result<SourceMetadata> {
    debug!("Falling back to GET metadata for {url}");
    let response = client
        .get(url.as_str())
        .header(header::RANGE, "bytes=0-0")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .with_context(|| format!("GET fallback failed for {url}"))?;

    let status = response.status();
    let response = response
        .error_for_status()
        .with_context(|| format!("GET fallback returned error status {} for {url}", status))?;

    build_metadata(url, &response)
}

fn build_metadata(url: Url, response: &Response) -> Result<SourceMetadata> {
    let headers = response.headers();

    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|len| len.to_str().ok())
        .and_then(|len| len.parse::<u64>().ok());

    let content_length = content_length
        .or_else(|| parse_content_range(headers.get(header::CONTENT_RANGE)))
        .with_context(|| format!("Missing Content-Length header for {url}"))?;

    let filename = infer_filename(&url, headers.get(header::CONTENT_DISPOSITION))?;

    Ok(SourceMetadata {
        url,
        content_length,
        filename,
    })
}

pub async fn stream(client: &Client, url: &Url) -> Result<Response> {
    let response = client
        .get(url.clone())
        .header(header::ACCEPT_ENCODING, "identity")
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .with_context(|| format!("GET request failed for {url}"))?;

    let status = response.status();
    response
        .error_for_status()
        .with_context(|| format!("GET request returned error status {} for {url}", status))
}

fn infer_filename(url: &Url, disposition: Option<&header::HeaderValue>) -> Result<String> {
    if let Some(value) = disposition.and_then(|hv| hv.to_str().ok()) {
        if let Some(name) = parse_content_disposition(value) {
            return Ok(sanitize_filename(&name));
        }
    }

    let path = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .unwrap_or_else(|| url.domain().unwrap_or("download").to_string());

    Ok(sanitize_filename(&path))
}

fn parse_content_disposition(header_value: &str) -> Option<String> {
    let mut filename = None;
    for part in header_value.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("filename*=") {
            if let Some(name) = parse_rfc5987(value) {
                return Some(name);
            }
        } else if let Some(value) = part.strip_prefix("filename=") {
            filename = strip_quotes(value.trim());
        }
    }

    filename
}

fn parse_rfc5987(value: &str) -> Option<String> {
    let mut sections = value.splitn(3, '\'');
    let _charset = sections.next()?;
    let _lang = sections.next();
    let encoded = sections.next()?;
    let decoded = percent_encoding::percent_decode_str(encoded).decode_utf8().ok()?;
    Some(decoded.to_string())
}

fn strip_quotes(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else if !trimmed.is_empty() {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn parse_content_range(value: Option<&header::HeaderValue>) -> Option<u64> {
    let header = value?.to_str().ok()?;
    let mut parts = header.split_whitespace();
    let unit = parts.next()?;
    if unit.to_ascii_lowercase() != "bytes" {
        return None;
    }
    let range = parts.next()?;
    let total = range.split('/').nth(1)?;
    if total == "*" {
        return None;
    }
    total.parse::<u64>().ok()
}
