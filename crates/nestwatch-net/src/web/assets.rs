// SPDX-License-Identifier: AGPL-3.0-or-later
//! Static files of the browser listener app (protocol.md §3.3, PRIV-1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hyper::body::Bytes;

/// Where the browser listener app's files come from.
#[derive(Debug, Clone)]
pub enum AssetSource {
    /// A directory on disk; files outside it are never served.
    Dir(PathBuf),
    /// Files compiled into the binary, keyed by relative path
    /// (`index.html`, `assets/app.wasm`; a leading `/` is ignored).
    Memory(HashMap<String, Bytes>),
}

impl AssetSource {
    /// An in-memory source from `(path, contents)` pairs.
    pub fn memory<P: Into<String>, B: Into<Bytes>>(
        files: impl IntoIterator<Item = (P, B)>,
    ) -> Self {
        Self::Memory(
            files
                .into_iter()
                .map(|(p, b)| (p.into().trim_start_matches('/').to_owned(), b.into()))
                .collect(),
        )
    }

    /// Loads the file at a sanitised relative path.
    pub(crate) async fn load(&self, rel: &str) -> Option<Bytes> {
        match self {
            Self::Memory(map) => map.get(rel).cloned(),
            Self::Dir(root) => load_from_dir(root, rel).await,
        }
    }
}

async fn load_from_dir(root: &Path, rel: &str) -> Option<Bytes> {
    let root = tokio::fs::canonicalize(root).await.ok()?;
    let path = tokio::fs::canonicalize(root.join(rel)).await.ok()?;
    // Symlinks must not lead outside the root.
    if !path.starts_with(&root) || !tokio::fs::metadata(&path).await.ok()?.is_file() {
        return None;
    }
    tokio::fs::read(&path).await.ok().map(Bytes::from)
}

/// Turns a request path into a safe relative file path: percent-decoded,
/// no `.`/`..`/empty segments, no backslashes or NUL. A trailing `/` maps to
/// `index.html`.
pub(crate) fn sanitize(path: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded);
    let mut parts = Vec::new();
    for seg in trimmed.split('/') {
        if seg == "." || seg == ".." || seg.contains(['\\', '\0']) || seg.contains(':') {
            return None;
        }
        if !seg.is_empty() {
            parts.push(seg);
        }
    }
    if trimmed.is_empty() || trimmed.ends_with('/') {
        parts.push("index.html");
    }
    Some(parts.join("/"))
}

/// MIME type by file extension.
pub(crate) fn mime_type(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_paths() {
        assert_eq!(sanitize("/").as_deref(), Some("index.html"));
        assert_eq!(sanitize("").as_deref(), Some("index.html"));
        assert_eq!(sanitize("/a/b.js").as_deref(), Some("a/b.js"));
        assert_eq!(sanitize("/a//b.js").as_deref(), Some("a/b.js"));
        assert_eq!(sanitize("/dir/").as_deref(), Some("dir/index.html"));
        assert_eq!(sanitize("/sp%20ace.txt").as_deref(), Some("sp ace.txt"));
        for bad in [
            "/../etc/passwd",
            "/a/%2e%2e/b",
            "/a/./b",
            "/a\\b",
            "/%00",
            "/c:/x",
            "/%ff",
        ] {
            assert_eq!(sanitize(bad), None, "{bad}");
        }
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_type("a/APP.WASM"), "application/wasm");
        assert_eq!(mime_type("x.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_type("noext"), "application/octet-stream");
    }
}
