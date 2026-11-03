// SPDX-License-Identifier: AGPL-3.0-or-later
//! Small helpers for backends that drive a helper program.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Finds `program` on `PATH`.
pub(crate) fn find_program(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs a command to completion with a timeout, returning stdout when it
/// exits successfully.
pub(crate) fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = out.join().unwrap_or_default();
    let _ = err.join();
    status.filter(|s| s.success()).map(|_| stdout)
}

/// Like [`run_with_timeout`] but returns stderr regardless of exit status
/// (for tools that print listings there).
pub(crate) fn run_stderr(cmd: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stderr = child.stderr.take()?;
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    err.join().ok()
}

/// Collects a child's stderr on a thread: keeps the last few lines and calls
/// `on_line` for each (lines split on `\n` or `\r`); lines for which it
/// returns `false` are not kept.
#[derive(Debug)]
pub(crate) struct StderrTail {
    lines: Arc<Mutex<Vec<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl StderrTail {
    const KEEP: usize = 8;

    pub(crate) fn spawn(
        stderr: impl Read + Send + 'static,
        mut on_line: impl FnMut(&str) -> bool + Send + 'static,
    ) -> Self {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::clone(&lines);
        let thread = std::thread::Builder::new()
            .name("nestwatch-stderr".into())
            .spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match read_line_any(&mut reader, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if !on_line(line) {
                        continue;
                    }
                    if let Ok(mut lines) = keep.lock() {
                        if lines.len() == Self::KEEP {
                            lines.remove(0);
                        }
                        lines.push(line.to_owned());
                    }
                }
            })
            .ok();
        Self { lines, thread }
    }

    /// The last lines seen, joined with `"; "`.
    pub(crate) fn tail(&self) -> String {
        join_lines(&self.lines)
    }

    /// A handle to read the tail from another thread.
    pub(crate) fn handle(&self) -> TailHandle {
        TailHandle(Arc::clone(&self.lines))
    }

    /// Waits for the child's stderr to close.
    pub(crate) fn join(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Shared view of a [`StderrTail`].
#[derive(Debug, Clone)]
pub(crate) struct TailHandle(Arc<Mutex<Vec<String>>>);

impl TailHandle {
    /// The last lines seen, joined with `"; "`.
    pub(crate) fn tail(&self) -> String {
        join_lines(&self.0)
    }
}

fn join_lines(lines: &Mutex<Vec<String>>) -> String {
    lines.lock().map(|l| l.join("; ")).unwrap_or_default()
}

/// Reads up to and including the next `\n` or `\r`.
fn read_line_any(reader: &mut impl BufRead, out: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut total = 0;
    loop {
        let (done, used) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(total);
            }
            match available.iter().position(|&b| b == b'\n' || b == b'\r') {
                Some(i) => {
                    out.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                }
                None => {
                    out.extend_from_slice(available);
                    (false, available.len())
                }
            }
        };
        reader.consume(used);
        total += used;
        if done {
            return Ok(total);
        }
    }
}

/// Kills and reaps a child process.
pub(crate) fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_cr_and_lf() {
        let mut r = BufReader::new(&b"a\rb\nc"[..]);
        let mut out = Vec::new();
        assert_eq!(read_line_any(&mut r, &mut out).unwrap(), 2);
        assert_eq!(out, b"a\r");
        out.clear();
        read_line_any(&mut r, &mut out).unwrap();
        assert_eq!(out, b"b\n");
        out.clear();
        read_line_any(&mut r, &mut out).unwrap();
        assert_eq!(out, b"c");
        out.clear();
        assert_eq!(read_line_any(&mut r, &mut out).unwrap(), 0);
    }

    #[test]
    fn stderr_tail_keeps_last_lines() {
        let data: Vec<u8> = (0..20)
            .flat_map(|i| format!("line {i}\r").into_bytes())
            .collect();
        let count = Arc::new(Mutex::new(0));
        let c = Arc::clone(&count);
        let mut tail = StderrTail::spawn(std::io::Cursor::new(data), move |_| {
            *c.lock().unwrap() += 1;
            true
        });
        tail.join();
        assert_eq!(*count.lock().unwrap(), 20);
        assert!(tail.tail().starts_with("line 12; "));
        assert!(tail.tail().ends_with("line 19"));
    }
}
