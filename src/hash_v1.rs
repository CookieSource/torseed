use sha1::{Digest, Sha1};

/// Streaming SHA-1 piece hasher for BitTorrent v1.
pub struct V1Hasher {
    piece_length: usize,
    current_len: usize,
    hasher: Sha1,
    pieces: Vec<u8>,
}

impl V1Hasher {
    pub fn new(piece_length: usize) -> Self {
        Self {
            piece_length,
            current_len: 0,
            hasher: Sha1::new(),
            pieces: Vec::new(),
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let remaining = self.piece_length - self.current_len;
            let to_take = remaining.min(data.len());
            let chunk = &data[..to_take];
            self.hasher.update(chunk);
            self.current_len += to_take;
            if self.current_len == self.piece_length {
                self.flush_piece();
            }
            data = &data[to_take..];
        }
    }

    pub fn finalize(mut self) -> Vec<u8> {
        if self.current_len > 0 {
            self.flush_piece();
        }
        self.pieces
    }

    fn flush_piece(&mut self) {
        let hasher = std::mem::take(&mut self.hasher);
        let digest = hasher.finalize();
        self.pieces.extend_from_slice(&digest);
        self.hasher = Sha1::new();
        self.current_len = 0;
    }
}
