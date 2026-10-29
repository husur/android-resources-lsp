use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The path of a `file:` URI, percent-decoded. On Windows, `file:///C:/x` gives `C:/x`.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut bytes = encoded.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'%' {
            let hex = [*bytes.next()?, *bytes.next()?];
            decoded.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            decoded.push(byte);
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => &path,
    };
    Some(PathBuf::from(path))
}

pub fn from_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from(if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(byte.into());
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}
