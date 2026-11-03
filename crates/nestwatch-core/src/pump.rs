// SPDX-License-Identifier: AGPL-3.0-or-later
//! Drives one sans-I/O session over one link (architecture.md §3): bytes in
//! → session → `poll_transmit` → link, timers from `poll_timeout`.
//!
//! At most one write is in flight; the next record is encrypted only when
//! the link has accepted the previous bytes, so socket backpressure reaches
//! the session's priority queues (protocol.md §8).

use std::time::Duration;

use nestwatch_net::{Link, LinkKind, LinkWriter, NetError};
use nestwatch_proto::frame::WsKind;
use tokio::sync::mpsc;

use crate::clock::Clock;
use crate::rt::{self, BoxFuture};

/// Stream bytes handed to a TCP link per write at most.
const TCP_CHUNK: usize = 16 * 1024;
/// Time allowed for the final flush and close of a link.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// The I/O side of a station or listener session.
pub(crate) trait Driven {
    fn feed(&mut self, now: u64, kind: LinkKind, data: &[u8]);
    fn feed_text(&mut self, now: u64);
    fn lost(&mut self, now: u64);
    fn transmit(&mut self, now: u64, max: usize) -> Option<Vec<u8>>;
    fn deadline(&self) -> Option<u64>;
    fn timeout(&mut self, now: u64);
    fn finished(&self) -> bool;
}

macro_rules! impl_driven {
    ($ty:ty $(, $g:ident : $b:path)?) => {
        impl$(<$g: $b>)? Driven for $ty {
            fn feed(&mut self, now: u64, kind: LinkKind, data: &[u8]) {
                match kind {
                    LinkKind::Stream => self.handle_bytes(now, data),
                    LinkKind::Messages => self.handle_ws_message(now, WsKind::Binary, data),
                }
            }
            fn feed_text(&mut self, now: u64) {
                self.handle_ws_message(now, WsKind::Text, &[]);
            }
            fn lost(&mut self, now: u64) {
                self.handle_connection_lost(now);
            }
            fn transmit(&mut self, now: u64, max: usize) -> Option<Vec<u8>> {
                self.poll_transmit(now, max)
            }
            fn deadline(&self) -> Option<u64> {
                self.poll_timeout()
            }
            fn timeout(&mut self, now: u64) {
                self.handle_timeout(now);
            }
            fn finished(&self) -> bool {
                self.is_finished()
            }
        }
    };
}

impl_driven!(nestwatch_session::ListenerSession);
#[cfg(feature = "native")]
impl_driven!(nestwatch_session::StationSession<P>, P: nestwatch_session::StationPolicy);

type Inflight = BoxFuture<(LinkWriter, Result<(), NetError>)>;

/// Runs `session` over `link` until the session is finished. `on_cmd`
/// applies commands from the engine; `after` runs after every step to drain
/// the session's events.
pub(crate) async fn drive<S, C>(
    session: &mut S,
    link: Link,
    clock: &dyn Clock,
    cmds: &mut mpsc::UnboundedReceiver<C>,
    mut on_cmd: impl FnMut(&mut S, C),
    mut after: impl FnMut(&mut S),
) where
    S: Driven,
{
    let kind = link.kind();
    let (mut reader, writer) = link.split();
    let mut writer = Some(writer);
    let mut inflight: Option<Inflight> = None;
    let mut reader_open = true;
    let mut cmds_open = true;
    loop {
        after(session);
        if inflight.is_none() {
            let max = match kind {
                LinkKind::Stream => TCP_CHUNK,
                LinkKind::Messages => usize::MAX,
            };
            if let Some(bytes) = session.transmit(clock.now_ms(), max) {
                let mut w = writer.take().expect("writer present when idle");
                inflight = Some(Box::pin(async move {
                    let r = w.send(&bytes).await;
                    (w, r)
                }));
            } else if session.finished() {
                break;
            }
        }
        let wait = session
            .deadline()
            .map(|d| Duration::from_millis(d.saturating_sub(clock.now_ms())));
        tokio::select! {
            (w, res) = async { inflight.as_mut().expect("checked").await }, if inflight.is_some() => {
                inflight = None;
                writer = Some(w);
                if res.is_err() {
                    session.lost(clock.now_ms());
                }
            }
            r = reader.recv(), if reader_open => match r {
                Ok(Some(bytes)) => session.feed(clock.now_ms(), kind, &bytes),
                Err(NetError::TextMessage) => session.feed_text(clock.now_ms()),
                Ok(None) | Err(_) => {
                    reader_open = false;
                    session.lost(clock.now_ms());
                }
            },
            () = rt::sleep(wait.unwrap_or_default()), if wait.is_some() => {
                session.timeout(clock.now_ms());
            }
            c = cmds.recv(), if cmds_open => match c {
                Some(c) => on_cmd(session, c),
                None => cmds_open = false,
            },
            else => break,
        }
    }
    after(session);
    let close = async move {
        let mut w = match inflight {
            Some(f) => f.await.0,
            None => writer.expect("writer present when idle"),
        };
        let _ = w.close().await;
    };
    let _ = rt::timeout(CLOSE_TIMEOUT, close).await;
}
