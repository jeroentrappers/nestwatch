// SPDX-License-Identifier: AGPL-3.0-or-later
//! Static files: the landing page, the browser listener app and downloads
//! (rendezvous.md §6, §8).

use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use tokio::io::AsyncReadExt;

use crate::http::Body;

/// Bytes read per body chunk when streaming a file.
const CHUNK: usize = 64 * 1024;

/// Turns a request path (below the mount point) into a safe relative path:
/// percent-decoded, no `.`/`..`/empty segments, no backslashes, NUL or
/// `:`. A trailing `/` (or nothing) maps to `index.html`.
pub(crate) fn sanitize(path: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded);
    let mut parts = Vec::new();
    for seg in trimmed.split('/') {
        if seg == "."
            || seg == ".."
            || seg.contains(['\\', '\0', ':'])
            || seg.starts_with('.') && !seg.is_empty() && seg != ".well-known"
        {
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

/// MIME type by extension.
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
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "apk" => "application/vnd.android.package-archive",
        "dmg" => "application/x-apple-diskimage",
        "msi" => "application/x-msi",
        "deb" => "application/vnd.debian.binary-package",
        "rpm" => "application/x-rpm",
        "appimage" => "application/vnd.appimage",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "sig" | "asc" => "application/pgp-signature",
        "sha256" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// An opened file ready to serve.
pub(crate) struct Opened {
    pub(crate) file: tokio::fs::File,
    pub(crate) len: u64,
    /// Weak validator from length and modification time.
    pub(crate) etag: String,
    pub(crate) rel: String,
}

/// Opens `rel` under `root` if it is a regular file inside `root` (symlinks
/// must not lead out).
pub(crate) async fn open(root: &Path, rel: &str) -> Option<Opened> {
    let root: PathBuf = tokio::fs::canonicalize(root).await.ok()?;
    let path = tokio::fs::canonicalize(root.join(rel)).await.ok()?;
    if !path.starts_with(&root) {
        return None;
    }
    let meta = tokio::fs::metadata(&path).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let file = tokio::fs::File::open(&path).await.ok()?;
    Some(Opened {
        file,
        len: meta.len(),
        etag: format!("W/\"{:x}-{:x}\"", meta.len(), mtime),
        rel: rel.to_owned(),
    })
}

/// Streams the file in chunks.
pub(crate) fn body(file: tokio::fs::File) -> Body {
    let s = stream::unfold(Some(file), |state| async move {
        let mut f = state?;
        let mut buf = vec![0; CHUNK];
        match f.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, io::Error>(Frame::data(Bytes::from(buf))), Some(f)))
            }
            Err(e) => Some((Err(e), None)),
        }
    });
    StreamBody::new(s).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_paths() {
        assert_eq!(sanitize("/").as_deref(), Some("index.html"));
        assert_eq!(sanitize("").as_deref(), Some("index.html"));
        assert_eq!(sanitize("/a/b.js").as_deref(), Some("a/b.js"));
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
            "/.git/config",
            "/.env",
        ] {
            assert_eq!(sanitize(bad), None, "{bad}");
        }
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_type("a/APP.WASM"), "application/wasm");
        assert_eq!(
            mime_type("nestwatch.apk"),
            "application/vnd.android.package-archive"
        );
        assert_eq!(mime_type("noext"), "application/octet-stream");
    }
}
