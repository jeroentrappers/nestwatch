# Media specification

## 1. Audio pipeline

```
mic → resample to 16 kHz mono → gain / AGC → level meter → encoder → per-listener gate → session queue
session → jitter buffer (per station) → decoder / concealment → volume → mixer → speaker
```

- Internal capture format: 16 kHz, mono, 16-bit signed PCM, frames of 20 ms
  (320 samples). Implementations resample from the device rate.
- **Level**: the RMS of each 20 ms frame in dBFS
  (`20·log10(rms / 32768)`, floor `-90`), measured **after the manual gain
  step and before AGC**. This **room level** is what meters, `level` records
  and sound activation use, so a threshold keeps its meaning while the AGC
  adapts.

### 1.1 Gain and automatic gain control

- Manual gain steps: 0, +6, +12, +18 dB.
- AGC (default on) shapes only what listeners hear. It tracks the room's
  noise floor (following drops immediately and rises at most 0.5 dB/s) and
  treats a frame as a **sound event** when it is at least 12 dB above that
  floor and above `-70 dBFS`. Only events raise the gain, slowly (at most
  +3 dB per second, up to +24 dB), towards a level of `-24 dBFS`; any frame
  that would exceed the target lowers it immediately (attack within 50 ms).
  Between events the gain holds, so a quiet or steady room is never pumped up,
  and frames below `-70 dBFS` are never amplified.

## 2. Audio codecs

| Id | Codec | Status | Notes |
|----|-------|--------|-------|
| 1 | Opus | SHOULD | RFC 6716, mono, 48 kHz RTP clock, 16 kHz audio bandwidth (wideband), 20 ms, VBR target 24 kbit/s, in-band FEC on |
| 2 | IMA ADPCM, 16 kHz | MUST | §2.1; 64 kbit/s; trivial to implement everywhere including the browser |

Every implementation MUST support codec 2 for encoding and decoding.

### 2.1 Codec 2 bitstream

One frame encodes 320 samples:

```
predictor   i16 (big-endian)  first sample of the frame, stored exactly
step_index  u8                0..88
reserved    u8                0
codes       160 bytes         samples 1..319 as 4-bit IMA codes, high nibble first;
                              the final low nibble is 0 and ignored
```

Encoding and decoding follow the IMA/DVI ADPCM algorithm with the standard
89-entry step-size table and the standard index-adjustment table
(`-1,-1,-1,-1,2,4,6,8` for code magnitudes 0..7). The encoder state at the
start of each frame is reset from the header, so every frame decodes
independently and a lost frame never corrupts the next.

### 2.2 Concealment

- Opus: use the decoder's own loss concealment and FEC.
- Codec 2: repeat the last decoded frame attenuated by 6 dB per repetition,
  for at most 3 frames, then silence.

## 3. Sound activation (station side, per listener)

Each listener session has a **gate** configured by its `listen` message.

1. The station keeps a ring buffer of the last 2 seconds of **encoded**
   frames per stream (shared by all sessions).
2. In `continuous` mode every frame goes to the session.
3. In `activated` mode, the gate opens when a frame's level is at or above
   the threshold. On opening, the station first queues the buffered frames
   covering the pre-roll (flag `pre-roll`), then the triggering frame (flag
   `first frame of a burst`), and sends `activity {open: true}`.
4. While open, every frame goes out. Each frame at or above the threshold
   re-arms the hold timer. When the hold time passes without such a frame,
   the gate closes and the station sends `activity {open: false}`.
5. Pre-roll frames are dropped first under backpressure.

The gate MUST NOT introduce extra latency to frames sent while open.

## 4. Listener playback

### 4.1 Jitter buffer (per station)

- Orders frames by `seq`, drops duplicates and frames older than the playout
  point.
- Target delay adapts between 60 ms and 300 ms from measured inter-arrival
  jitter (exponentially weighted mean deviation, target = 3× jitter + 20 ms),
  growing immediately and shrinking slowly (at most 20 ms per 10 s).
- **Bursts**: frames flagged `pre-roll` and the following burst arrive faster
  than real time. The buffer MUST NOT count them as jitter. It plays them
  back at normal speed, then skims excess delay by dropping concealable
  silence or shortening pauses until back at target.
- A gap in `seq` while the gate is open is concealed (§2.2). A gap at the
  start of a burst is normal and is not concealed.

### 4.2 Mixer

- Mixes all unmuted stations at their volumes. With solo active, only the
  solo station is audible, but every station's level and alerts keep working.
- Uses a soft limiter (linear up to `-3 dBFS`, smooth knee above) so several
  loud stations never clip harshly.
- While the user talks to a station, that station's playback is muted
  (product.md FR-TB-2).

### 4.3 Talk-back capture

The listener's microphone goes through the same capture path (16 kHz, gain
fixed at 0 dB, platform echo cancellation and noise suppression on where
available) and is sent as stream 1 only while `talk-state` is `granted`.

## 5. Video

- Codec 1 is baseline JPEG (ISO/IEC 10918-1), YCbCr 4:2:0. Each video frame
  is an independent JPEG image.
- Default request: 640×480, 10 fps. A station never exceeds the listener's
  `max_w`, `max_h`, `max_fps`, and preserves the camera's aspect ratio.
- Adaptation: start at quality 70. Each time a queued frame is replaced
  (protocol.md §8), lower the frame rate by one step
  (15, 10, 7, 5, 3, 2, 1 fps) and, at 3 fps or below, lower quality in steps
  of 10 down to 30. After 5 seconds without replacement, step back up one
  level.
- Night image (`video-options night: true`): grayscale, with a gamma curve
  that lifts shadows; never use the device torch.
- The camera starts when the first listener sends `video-start` and stops
  within 1 second after the last one sends `video-stop` or disconnects.
- Several listeners share one capture at the largest requested size; each
  gets its own frame rate and adaptation.

## 6. Clarifications

Resolved implementation questions. These statements are normative and take
precedence over looser wording above.

- **MEDIA-1** (media.md §1.1). Superseded: the AGC is event-based relative to a tracked noise floor (§1.1), and levels for meters, `level` records and the gate are measured before AGC (§1). The earlier rule that every frame above -70 dBFS counts as an event pumped quiet rooms above listener thresholds.
- **MEDIA-2** (media.md §1.1). Frames below -70 dBFS pass at unity gain and do not change the AGC gain.
- **MEDIA-3** (media.md §1.1). Exact attack behaviour ("within 50 ms") **Resolution:** Gain decreases apply to the whole frame that triggered them (≤ 20 ms); increases ramp linearly inside a frame and are limited to +0.06 dB per 20 ms frame (3 dB/s).
- **MEDIA-4** (media.md §2.1). How does the encoder choose `step_index`, and must decoders reject a non-zero reserved byte? **Resolution:** Reference encoder: predictor = sample 0; step_index = the index (0..88) minimising squared coding error over samples 1..31, ties to the lowest; no state carried between frames. Decoders ignore the reserved byte and reject step_index > 88 or a length other than 164.
- **MEDIA-5** (media.md §2.2). Is "6 dB per repetition" exact or a halving? **Resolution:** Exact: gains 10^(−6n/20) (0.501, 0.251, 0.126), rounded to nearest.
- **MEDIA-6** (media.md §3, protocol.md §6.1). Where does `seq` start for a listener, which frames can be pre-roll, and how are pre-roll and hold times rounded to frames? **Resolution:** `seq` = captured frames since the gate (session) was created, starting at 0. Pre-roll covers ceil(preroll_ms / 20) frames but never frames captured before the session or frames already sent. A frame is held while (frames since last loud frame) × 20 ms ≤ hold_ms; the first frame beyond closes the gate and is not sent.
- **MEDIA-7** (media.md §3, protocol.md §6.1, §7.5). Flags and `activity` on mode switches and continuous mode **Resolution:** Pre-roll frames carry only the pre-roll flag; the trigger carries first-of-burst; in continuous mode any frame not directly following the last sent one (session start, mode switch) carries first-of-burst. Continuous mode sends no `activity`. Switching continuous → activated keeps the gate open with the hold armed at the last sent frame (it later closes with `activity {open: false}`); activated → continuous sends no `activity`.
- **MEDIA-8** (media.md §4.1). When does the faster-than-real-time burst end, what is "concealable silence", and what if a long loud burst has no silence? **Resolution:** Frames flagged pre-roll / first-of-burst and following frames arriving > 5 ms faster than real time are excluded from jitter. Target delay is measured from the smoothed mean transit (EW 1/16). Excess delay is skimmed one frame per pull from non-pre-roll frames at or below -50 dBFS or 12 dB below the recent average level; if excess persists 1 s without such frames, one frame is dropped every 60 ms. Target growth is applied immediately by inserting concealment frames. After 3 missing frames with nothing buffered the buffer goes idle (silence).
- **MEDIA-9** (media.md §4.2, product.md FR-TB-2). Is talk-back "muting" of the talked-to station full mute, and how is volume expressed? **Resolution:** Full mute with a 20 ms ramp; volume is linear 0..1; station levels are measured before volume, mute and solo. The limiter is a static curve: identity to -3 dBFS, tanh knee above.
- **MEDIA-10** (media.md §5). How do fps and quality steps combine on the ladder, where does it start for a `max_fps` not on the ladder, and which night gamma? **Resolution:** Each replacement moves one level: 15/70, 10/70, 7/70, 5/70, 3/60, 2/50, 1/40, 1/30 (quality drops on steps landing at ≤ 3 fps, then extra 1 fps levels to 30). The top level is the highest ladder rate ≤ max_fps (15 if above). One level up per 5 s without replacement. Night filter: BT.601 luma with tone curve out = in^0.6. YUV input is treated as full-range (JFIF) BT.601.
