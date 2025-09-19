use std::borrow::Cow;
use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use bendy::encoding::ToBencode;
use bendy::value::Value;
use sha1::{Digest as Sha1DigestTrait, Sha1};
use sha2::Sha256;

use crate::hash_v2::V2Summary;

#[derive(Debug, Clone)]
pub struct BuildInput {
    pub name: String,
    pub length: u64,
    pub piece_length: u32,
    pub pieces: Vec<u8>,
    pub trackers: Vec<String>,
    pub webseeds: Vec<String>,
    pub creation_date: i64,
    pub created_by: String,
    pub v2: Option<V2Summary>,
}

#[derive(Debug, Clone)]
pub struct Metainfo {
    pub torrent: Vec<u8>,
    pub infohash_v1: Option<[u8; 20]>,
    pub infohash_v2: Option<[u8; 32]>,
}

type Dict = BTreeMap<Cow<'static, [u8]>, Value<'static>>;

pub fn build(input: &BuildInput) -> Result<Metainfo> {
    if input.trackers.is_empty() {
        bail!("At least one tracker is required");
    }

    let info_full = build_info_full(input)?;
    let info_v1 = build_info_v1(input)?;
    let info_v2 = build_info_v2(input)?;

    let infohash_v1 = Some(
        Sha1::digest(
            &info_v1
                .to_bencode()
                .map_err(|err| anyhow!("Failed to encode v1 info dictionary: {err}"))?,
        )
        .into(),
    );
    let infohash_v2 = if input.v2.is_some() {
        Some(
            Sha256::digest(
                &info_v2
                    .to_bencode()
                    .map_err(|err| anyhow!("Failed to encode v2 info dictionary: {err}"))?,
            )
            .into(),
        )
    } else {
        None
    };

    let torrent = build_torrent_root(input, info_full)?;

    Ok(Metainfo {
        torrent,
        infohash_v1,
        infohash_v2,
    })
}

fn build_torrent_root(input: &BuildInput, info: Value<'static>) -> Result<Vec<u8>> {
    let mut root: Dict = BTreeMap::new();
    root.insert(key("announce"), bytes(input.trackers[0].clone()));

    let announce_tier: Vec<Value<'static>> = input
        .trackers
        .iter()
        .map(|t| bytes(t.clone()))
        .collect();
    root.insert(
        key("announce-list"),
        Value::List(vec![Value::List(announce_tier)]),
    );

    root.insert(key("created by"), bytes(input.created_by.clone()));
    root.insert(key("creation date"), Value::Integer(input.creation_date));
    root.insert(key("info"), info);

    let webseed_list: Vec<Value<'static>> = input
        .webseeds
        .iter()
        .map(|ws| bytes(ws.clone()))
        .collect();
    root.insert(key("url-list"), Value::List(webseed_list));

    Value::Dict(root)
        .to_bencode()
        .map_err(|err| anyhow!("Failed to encode root dictionary: {err}"))
}

fn build_info_full(input: &BuildInput) -> Result<Value<'static>> {
    let mut dict = info_v1_map(input)?;
    if let Some(v2) = &input.v2 {
        dict.extend(info_v2_map(input, v2)?);
    }
    Ok(Value::Dict(dict))
}

fn build_info_v1(input: &BuildInput) -> Result<Value<'static>> {
    Ok(Value::Dict(info_v1_map(input)?))
}

fn build_info_v2(input: &BuildInput) -> Result<Value<'static>> {
    match &input.v2 {
        Some(v2) => Ok(Value::Dict(info_v2_map(input, v2)?)),
        None => Ok(Value::Dict(BTreeMap::new())),
    }
}

fn info_v1_map(input: &BuildInput) -> Result<Dict> {
    let mut dict = BTreeMap::new();
    dict.insert(key("length"), Value::Integer(i64_from_u64(input.length)?));
    dict.insert(key("name"), bytes(input.name.clone()));
    dict.insert(
        key("piece length"),
        Value::Integer(i64::from(input.piece_length)),
    );
    dict.insert(key("pieces"), bytes(input.pieces.clone()));
    Ok(dict)
}

fn info_v2_map(input: &BuildInput, v2: &V2Summary) -> Result<Dict> {
    let mut dict = BTreeMap::new();
    dict.insert(key("meta version"), Value::Integer(2));
    dict.insert(key("name"), bytes(input.name.clone()));
    dict.insert(
        key("piece length"),
        Value::Integer(i64::from(input.piece_length)),
    );
    dict.insert(key("file tree"), build_file_tree(input, v2)?);
    dict.insert(key("piece layers"), build_piece_layers(v2));
    Ok(dict)
}

fn build_file_tree(input: &BuildInput, v2: &V2Summary) -> Result<Value<'static>> {
    let mut leaf = BTreeMap::new();
    leaf.insert(key("length"), Value::Integer(i64_from_u64(input.length)?));
    leaf.insert(key("pieces root"), bytes(v2.pieces_root.to_vec()));

    let mut file_entry = BTreeMap::new();
    file_entry.insert(Cow::Owned(Vec::new()), Value::Dict(leaf));

    let mut tree = BTreeMap::new();
    tree.insert(key(&input.name), Value::Dict(file_entry));

    Ok(Value::Dict(tree))
}

fn build_piece_layers(v2: &V2Summary) -> Value<'static> {
    let mut dict = BTreeMap::new();
    dict.insert(Cow::Owned(v2.pieces_root.to_vec()), bytes(v2.piece_layers.clone()));
    Value::Dict(dict)
}

fn bytes(data: impl Into<Vec<u8>>) -> Value<'static> {
    Value::Bytes(Cow::Owned(data.into()))
}

fn key(input: &str) -> Cow<'static, [u8]> {
    Cow::Owned(input.as_bytes().to_vec())
}

fn i64_from_u64(value: u64) -> Result<i64> {
    i64::try_from(value).context("value exceeds i64 range")
}
