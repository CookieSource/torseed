const DEFAULT_NAME: &str = "download";
const SAFE_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-";

/// Choose a v1 piece length that keeps the number of pieces reasonable (~16k max).
pub fn choose_piece_length(size: u64) -> usize {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    let length_bytes = if size <= 128 * MB {
        256 * KB
    } else if size <= 1 * GB {
        512 * KB
    } else if size <= 4 * GB {
        1 * MB
    } else if size <= 16 * GB {
        2 * MB
    } else if size <= 64 * GB {
        4 * MB
    } else {
        8 * MB
    };

    length_bytes as usize
}

/// Sanitizes a suggested file name for use on disk.
pub fn sanitize_filename(input: &str) -> String {
    let candidate = input.trim();
    let mut result = String::with_capacity(candidate.len());

    for ch in candidate.chars() {
        if ch == '.' && (result.is_empty() || result == ".") {
            continue;
        }
        if ch.is_ascii() && SAFE_CHARS.contains(&(ch as u8)) {
            result.push(ch);
        } else {
            result.push('_');
        }
    }

    if result.is_empty() {
        DEFAULT_NAME.to_string()
    } else {
        result
    }
}

/// Formats bytes using GiB/MiB/...
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[(&str, u64)] = &[
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
    ];

    for (label, size) in UNITS {
        if bytes >= *size {
            let value = bytes as f64 / *size as f64;
            return format!("{value:.2} {label}");
        }
    }

    format!("{} B", bytes)
}
