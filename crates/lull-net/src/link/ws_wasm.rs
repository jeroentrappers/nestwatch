// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket message links in the browser (protocol.md §3.3), on `web-sys`.

use std::fmt;
use std::rc::Rc;

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt;
use js_sys::{ArrayBuffer, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

use super::{LinkInfo, LinkKind, check_len};
use crate::NetError;

/// `send` waits while the browser buffers more than this many bytes.
const HIGH_WATER: u32 = 256 * 1024;
/// Poll interval while waiting for the send buffer to drain.
const DRAIN_POLL_MS: i32 = 5;

enum WsEvent {
    Open,
    Binary(Vec<u8>),
    Text,
    Error,
    Close,
}

/// Owns the socket and its callbacks; dropping it detaches and closes.
struct Socket {
    ws: WebSocket,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onerror(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

fn js_err(v: JsValue) -> NetError {
    NetError::Browser(v.as_string().unwrap_or_else(|| format!("{v:?}")))
}

/// Opens a WebSocket with `binaryType = "arraybuffer"` and waits for it to
/// open or fail.
pub(crate) async fn open(url: &str) -> Result<crate::Link, NetError> {
    let ws = WebSocket::new(url).map_err(js_err)?;
    ws.set_binary_type(BinaryType::Arraybuffer);
    let (tx, mut rx) = unbounded::<WsEvent>();

    let send = |tx: &UnboundedSender<WsEvent>, e| {
        let _ = tx.unbounded_send(e);
    };
    let t = tx.clone();
    let on_open = Closure::<dyn FnMut(Event)>::new(move |_| send(&t, WsEvent::Open));
    let t = tx.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let data = e.data();
        let ev = match data.dyn_into::<ArrayBuffer>() {
            Ok(buf) => WsEvent::Binary(Uint8Array::new(&buf).to_vec()),
            Err(_) => WsEvent::Text,
        };
        send(&t, ev);
    });
    let t = tx.clone();
    let on_error = Closure::<dyn FnMut(Event)>::new(move |_| send(&t, WsEvent::Error));
    let t = tx;
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_| send(&t, WsEvent::Close));
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    let socket = Rc::new(Socket {
        ws,
        _on_open: on_open,
        _on_message: on_message,
        _on_error: on_error,
        _on_close: on_close,
    });

    match rx.next().await {
        Some(WsEvent::Open) => {}
        _ => return Err(NetError::Browser("WebSocket connection failed".into())),
    }
    let info = LinkInfo {
        kind: LinkKind::Messages,
        peer: None,
        local: None,
        tls: url.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("wss:")),
    };
    Ok(crate::Link::Messages(MessageLink {
        reader: MessageReader {
            rx,
            _socket: socket.clone(),
        },
        writer: MessageWriter { socket },
        info,
    }))
}

async fn sleep_ms(ms: i32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        let global = js_sys::global();
        let set_timeout = Reflect::get(&global, &JsValue::from_str("setTimeout"))
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok());
        match set_timeout {
            Some(f) => {
                let _ = f.call2(&global, &resolve, &JsValue::from(ms));
            }
            None => {
                let _ = resolve.call0(&JsValue::NULL);
            }
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// A browser WebSocket carrying one Noise message per binary message.
#[derive(Debug)]
pub struct MessageLink {
    reader: MessageReader,
    writer: MessageWriter,
    info: LinkInfo,
}

impl MessageLink {
    /// TLS state (addresses are not known in the browser).
    pub fn info(&self) -> &LinkInfo {
        &self.info
    }

    /// See [`crate::Link::recv`].
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        self.reader.recv().await
    }

    /// See [`crate::Link::send`].
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.writer.send(data).await
    }

    /// See [`crate::Link::close`].
    pub async fn close(&mut self) -> Result<(), NetError> {
        self.writer.close().await
    }

    /// Splits into read and write halves.
    pub fn split(self) -> (MessageReader, MessageWriter) {
        (self.reader, self.writer)
    }
}

/// Read half of a browser [`MessageLink`].
pub struct MessageReader {
    rx: UnboundedReceiver<WsEvent>,
    _socket: Rc<Socket>,
}

impl fmt::Debug for MessageReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageReader").finish_non_exhaustive()
    }
}

impl MessageReader {
    /// Next binary message; `Ok(None)` after a close. Cancel-safe.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        loop {
            return match self.rx.next().await {
                Some(WsEvent::Binary(b)) => Ok(Some(b)),
                Some(WsEvent::Text) => Err(NetError::TextMessage),
                Some(WsEvent::Error) => Err(NetError::Browser("WebSocket error".into())),
                Some(WsEvent::Open) => continue,
                Some(WsEvent::Close) | None => Ok(None),
            };
        }
    }
}

/// Write half of a browser [`MessageLink`].
pub struct MessageWriter {
    socket: Rc<Socket>,
}

impl fmt::Debug for MessageWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageWriter").finish_non_exhaustive()
    }
}

impl MessageWriter {
    /// Sends one binary message, then waits while the browser's send
    /// buffer holds more than 256 KiB.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        check_len(data.len())?;
        let ws = &self.socket.ws;
        if ws.ready_state() != WebSocket::OPEN {
            return Err(NetError::Closed);
        }
        ws.send_with_u8_array(data).map_err(js_err)?;
        while ws.buffered_amount() > HIGH_WATER {
            if ws.ready_state() != WebSocket::OPEN {
                return Err(NetError::Closed);
            }
            sleep_ms(DRAIN_POLL_MS).await;
        }
        Ok(())
    }

    /// Starts the WebSocket closing handshake.
    pub async fn close(&mut self) -> Result<(), NetError> {
        self.socket.ws.close().map_err(js_err)
    }
}
