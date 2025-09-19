use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};

const MAGNET_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub fn build_magnets(
    name: &str,
    trackers: &[String],
    webseeds: &[String],
    infohash_v1: Option<[u8; 20]>,
    infohash_v2: Option<[u8; 32]>,
) -> Vec<String> {
    let mut magnets = Vec::new();
    if let Some(hash) = infohash_v1 {
        magnets.push(build_btih(name, trackers, webseeds, &hash));
    }
    if let Some(hash) = infohash_v2 {
        magnets.push(build_btmh(name, trackers, webseeds, &hash));
    }
    magnets
}

fn build_btih(name: &str, trackers: &[String], webseeds: &[String], hash: &[u8; 20]) -> String {
    let mut magnet = format!("magnet:?xt=urn:btih:{}", hex::encode(hash));
    append_common(&mut magnet, name, trackers, webseeds);
    magnet
}

fn build_btmh(name: &str, trackers: &[String], webseeds: &[String], hash: &[u8; 32]) -> String {
    let mut magnet = String::from("magnet:?xt=urn:btmh:1220");
    magnet.push_str(&hex::encode(hash));
    append_common(&mut magnet, name, trackers, webseeds);
    magnet
}

fn append_common(magnet: &mut String, name: &str, trackers: &[String], webseeds: &[String]) {
    magnet.push_str("&dn=");
    magnet.push_str(&encode_component(name));

    for tracker in trackers {
        magnet.push_str("&tr=");
        magnet.push_str(&encode_component(tracker));
    }

    for ws in webseeds {
        magnet.push_str("&ws=");
        magnet.push_str(&encode_component(ws));
    }
}

fn encode_component(value: &str) -> String {
    percent_encode(value.as_bytes(), MAGNET_ENCODE_SET).to_string()
}
