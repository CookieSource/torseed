use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use anyhow::Result;
use sha2::{Digest, Sha256};
use tempfile::tempfile;

const LEAF_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct V2Summary {
    pub pieces_root: [u8; 32],
    pub piece_layers: Vec<u8>,
}

pub struct V2Hasher {
    buffer: Vec<u8>,
    leaf_writer: BufWriter<std::fs::File>,
    leaf_count: usize,
    total_bytes: u64,
}

impl V2Hasher {
    pub fn new() -> Result<Self> {
        let temp = tempfile()?;
        Ok(Self {
            buffer: Vec::with_capacity(LEAF_SIZE),
            leaf_writer: BufWriter::new(temp),
            leaf_count: 0,
            total_bytes: 0,
        })
    }

    pub fn update(&mut self, mut data: &[u8]) -> Result<()> {
        self.total_bytes += data.len() as u64;

        while !data.is_empty() {
            let needed = LEAF_SIZE - self.buffer.len();
            let take = needed.min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];

            if self.buffer.len() == LEAF_SIZE {
                self.flush_leaf()?;
            }
        }

        Ok(())
    }

    pub fn finalize(mut self, piece_length: usize) -> Result<V2Summary> {
        if !self.buffer.is_empty() {
            self.flush_leaf()?;
        }

        if self.leaf_count == 0 {
            let digest: [u8; 32] = Sha256::digest(&[]).into();
            self.write_leaf(&digest)?;
        }

        let mut file = self.leaf_writer.into_inner()?;
        file.seek(SeekFrom::Start(0))?;
        let leaves = read_leaves(&mut file, self.leaf_count)?;

        let piece_count = if self.total_bytes == 0 {
            0
        } else {
            ((self.total_bytes + piece_length as u64 - 1) / piece_length as u64) as usize
        };

        let piece_layers = if piece_count == 0 {
            Vec::new()
        } else {
            build_piece_layers(&leaves, piece_length, piece_count)
        };
        let pieces_root = merkle_root(&leaves);

        Ok(V2Summary {
            pieces_root,
            piece_layers,
        })
    }

    fn flush_leaf(&mut self) -> Result<()> {
        let digest: [u8; 32] = Sha256::digest(&self.buffer).into();
        self.write_leaf(&digest)?;
        self.buffer.clear();
        Ok(())
    }

    fn write_leaf(&mut self, digest: &[u8]) -> Result<()> {
        self.leaf_writer.write_all(digest)?;
        self.leaf_count += 1;
        Ok(())
    }

}

fn read_leaves(file: &mut std::fs::File, leaf_count: usize) -> Result<Vec<[u8; 32]>> {
    let mut leaves = Vec::with_capacity(leaf_count);
    let mut buf = [0u8; 32];
    for _ in 0..leaf_count {
        file.read_exact(&mut buf)?;
        leaves.push(buf);
    }
    Ok(leaves)
}

fn build_piece_layers(leaves: &[[u8; 32]], piece_length: usize, piece_count: usize) -> Vec<u8> {
    if leaves.is_empty() || piece_length == 0 || piece_count == 0 {
        return Vec::new();
    }

    let leaves_per_piece = (piece_length + LEAF_SIZE - 1) / LEAF_SIZE;
    let mut layers: Vec<u8> = Vec::with_capacity(piece_count * 32);
    let mut index = 0;
    for _ in 0..piece_count {
        if index >= leaves.len() {
            break;
        }
        let end = (index + leaves_per_piece).min(leaves.len());
        let root = merkle_root(&leaves[index..end]);
        layers.extend_from_slice(&root);
        index = end;
    }

    layers
}

fn merkle_root(nodes: &[[u8; 32]]) -> [u8; 32] {
    if nodes.is_empty() {
        return Sha256::digest(&[]).into();
    }

    let mut level: Vec<[u8; 32]> = nodes.to_vec();

    while level.len() > 1 {
        if level.len() % 2 == 1 {
            let last = *level.last().unwrap();
            level.push(last);
        }

        let mut next = Vec::with_capacity(level.len() / 2);
        for chunk in level.chunks_exact(2) {
            next.push(hash_pair(&chunk[0], &chunk[1]));
        }
        level = next;
    }

    level[0]
}

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}
