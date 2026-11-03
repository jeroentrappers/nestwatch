# Open questions

Implementers record here every place where the specs are ambiguous,
incomplete or contradictory, with the interpretation they chose. The spec
owner resolves each entry by changing the specs and removing it here.

| ID | Spec section | Question | Interim choice |
|----|--------------|----------|----------------|
| UI-1 | design/README.md §4 (`--level`, `--threshold`) | How does a dBFS level map to the 0..1 meter and ring position, and which thresholds can the user pick? | Linear in dB from −90 dBFS (0) to −10 dBFS (1), clamped. Thresholds −85..−10 dBFS in 1 dB steps (keyboard: arrows ±1, Page ±5, Home/End). |
| UI-2 | design/README.md §4 "Level ring" | "Tick marks the strictest listener threshold": highest or lowest threshold, and what about continuous listeners? | The lowest threshold of the connected sound-activated listeners (above it at least one of them hears the room); no tick without one. The ring says "sound" while sound activation is open for any sound-activated listener; continuous listeners do not count. `ConnectedListener` now reports `threshold_dbfs`. |
| UI-3 | product.md FR-PAIR-2, protocol.md §5.3 | Code pairing needs the station's `device_id` from mDNS, which a browser cannot browse. | The station's pairing sheet shows its device ID under the code; the browser form asks for it and dials the page's own `ws(s)://host/lull/1`. Native listeners take the ID and addresses from the selected nearby station. |
| UI-4 | security.md §5 "Browser" | IndexedDB with non-extractable Web Crypto keys, or another store? | `localStorage` of the station's origin, values base64 under `lull:<key>`. The Noise implementation needs the raw X25519 secret, so a non-extractable key cannot be used; both stores have the same origin scope, and `localStorage` matches the synchronous storage port (CORE-13) without a cache. |
| UI-5 | product.md FR-AU-2, design/README.md §4 "Calibration" | Where does "the last minute of room level" come from? | The listener records the station's `level` reports itself (one point per 200 ms, peak kept) while the station is live; kept in memory for 60 s, not persisted. |
| UI-6 | product.md §1 | When is the medical disclaimer acknowledged, and must it block the app? | It is shown on the welcome screen with the role choice (browser: an "I understand" button) and in About; choosing a role records the acknowledgement in the UI preferences. |
| UI-7 | platforms.md §2.5 | Which gesture unlocks browser audio? | The first pointer or key press anywhere creates and resumes a 16 kHz `AudioContext`; a "Tap to start listening" prompt stays visible until playback runs. |
| UI-8 | platforms.md §2.5, product.md FR-AL-1 | Browser alerts outside a secure context (plain `http://` LAN address) where the Notification API is unavailable. | Always an in-page notice and a short two-tone sound (even when muted, FR-AL-4); system notifications only in secure contexts after the user enables them. |
| UI-9 | platforms.md §2.5, media.md §4 | Talk-back capture rate in the browser. | The microphone runs in its own `AudioContext` at the device rate (some browsers refuse a `MediaStream` source in a 16 kHz context); blocks go to `push_talk_capture` with that rate. |
| UI-10 | protocol.md §3.3, platforms.md §2.4 | Where does a desktop station find the browser bundle it serves? | First of `$LULL_WEB_DIR`, `<exe dir>/web`, `<exe dir>/../share/lull/web`, the workspace's `target/lull-web`; otherwise a placeholder page that says the app is missing. |
| UI-11 | product.md FR-CAM-1 | How are camera frames shown? | The newest complete JPEG as an `img` with a `data:` URI (allowed by `img-src data:`); same on desktop and web. |
