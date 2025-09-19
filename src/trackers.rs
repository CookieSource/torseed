use std::{collections::HashSet, time::{Duration, Instant}};

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use rand::{seq::SliceRandom, thread_rng};
use reqwest::Client;
use tracing::{debug, info, warn};
use url::Url;

const FALLBACK_TRACKERS: &str = r"udp://tracker.opentrackr.org:1337/announce
udp://open.stealth.si:80/announce
udp://tracker.torrent.eu.org:451/announce
udp://tracker.nanoha.org:6969/announce
udp://tracker.moeking.me:6969/announce
udp://tracker.skynetcloud.site:6969/announce
udp://explodie.org:6969/announce
udp://tracker1.bt.moack.co.kr:80/announce
udp://tracker.bitsearch.to:1337/announce
udp://tracker2.dler.org:80/announce
udp://exodus.desync.com:6969/announce
udp://tracker.open-internet.nl:6969/announce
udp://tracker.filemail.com:6969/announce
udp://open.demonii.com:1337/announce
udp://tracker3.itzmx.com:6961/announce
udp://public.tracker.vraphim.com:6969/announce
udp://tracker4.itzmx.com:2710/announce
udp://tracker.theoks.net:6969/announce
udp://tracker.cyberia.is:6969/announce
udp://tracker-udp.gbitt.info:80/announce
udp://tracker.truenethosting.com:6969/announce
udp://tracker.dler.com:6969/announce
udp://tracker.internetwarriors.net:1337/announce
udp://tracker.skyts.net:6969/announce
udp://opentracker.i2p.rocks:6969/announce
udp://bt1.archive.org:6969/announce
udp://bt2.archive.org:6969/announce
http://tracker.opentrackr.org:1337/announce
http://tracker.files.fm:6969/announce
http://tracker.tiny-vps.com:6969/announce
http://tracker3.itzmx.com:6961/announce
http://tracker.torrent.eu.org:451/announce
http://retracker.sevstar.net:2710/announce
https://tracker.opentrackr.org:443/announce
https://tracker.gbitt.info:443/announce
https://tr.ready4.icu:443/announce
https://tracker.tamersunion.org:443/announce
https://tracker.imgoingto.icu:443/announce
https://tracker.renfei.net:443/announce
";

const TRACKER_SOURCES: &[&str] = &[
    "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_best.txt",
    "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_all.txt",
    "https://raw.githubusercontent.com/XIU2/TrackersListCollection/master/best.txt",
    "https://raw.githubusercontent.com/XIU2/TrackersListCollection/master/all.txt",
    "https://trackerslist.com/all.txt",
    "https://newtrackon.com/api/stable",
];

pub async fn gather_trackers(client: &Client) -> Result<Vec<String>> {
    let fallback = parse_tracker_block(FALLBACK_TRACKERS);
    if fallback.is_empty() {
        return Err(anyhow!("Fallback tracker list is empty"));
    }

    let mut aggregated = Vec::new();
    let mut seen = HashSet::new();

    for tracker in &fallback {
        if seen.insert(tracker.clone()) {
            aggregated.push(tracker.clone());
            if aggregated.len() >= 1000 {
                return Ok(aggregated);
            }
        }
    }

    let mut futures = FuturesUnordered::new();
    for &source_url in TRACKER_SOURCES {
        let client = client.clone();
        let source = source_url.to_string();
        futures.push(async move {
            let start = Instant::now();
            let result = tokio::time::timeout(Duration::from_secs(8), client.get(&source).send()).await;
            match result {
                Ok(Ok(response)) => {
                    match response.error_for_status() {
                        Ok(response) => match response.text().await {
                            Ok(text) => {
                                let trackers = parse_tracker_block(&text);
                                let elapsed = start.elapsed();
                                return Some((elapsed, trackers, source));
                            }
                            Err(err) => {
                                warn!("Tracker source {source} text decode failed: {err}");
                            }
                        },
                        Err(err) => {
                            warn!("Tracker source {source} returned error status: {err}");
                        }
                    }
                }
                Ok(Err(err)) => {
                    warn!("Tracker source {source} failed: {err}");
                }
                Err(_) => {
                    warn!("Tracker source {source} timed out");
                }
            }
            None
        });
    }

    let mut results = Vec::new();
    while let Some(Some(entry)) = futures.next().await {
        results.push(entry);
    }

    results.sort_by_key(|(elapsed, _, _)| *elapsed);

    for (elapsed, trackers, source) in results {
        debug!("tracker_source = {source}, elapsed = {:?}, discovered = {}", elapsed, trackers.len());
        let mut trackers = trackers;
        trackers.shuffle(&mut thread_rng());
        for tracker in trackers {
            if seen.insert(tracker.clone()) {
                aggregated.push(tracker);
                if aggregated.len() >= 1000 {
                    break;
                }
            }
        }
        if aggregated.len() >= 1000 {
            break;
        }
    }

    info!("Total trackers gathered: {}", aggregated.len());

    if aggregated.is_empty() {
        Err(anyhow!("No trackers available"))
    } else {
        Ok(aggregated)
    }
}

fn parse_tracker_block(block: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|line| normalize_tracker(line))
        .collect()
}

fn normalize_tracker(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let mut url = Url::parse(trimmed).ok()?;
    match url.scheme() {
        "udp" | "http" | "https" | "ws" | "wss" => {}
        _ => return None,
    }

    if let Some(host) = url.host_str() {
        let host_lower = host.to_ascii_lowercase();
        url.set_host(Some(&host_lower)).ok()?;
    } else {
        return None;
    }

    let scheme_lower = url.scheme().to_ascii_lowercase();
    url.set_scheme(&scheme_lower).ok()?;

    // Remove default port schemes like :80 for http, :443 for https when present
    if (url.scheme() == "http" && url.port() == Some(80)) || (url.scheme() == "https" && url.port() == Some(443)) {
        url.set_port(None).ok();
    }

    let mut normalized = url.to_string();
    if matches!(url.scheme(), "http" | "https")
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && normalized.ends_with('/')
    {
        normalized.pop();
    }

    Some(normalized)
}
