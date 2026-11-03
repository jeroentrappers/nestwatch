# Media specification

## 1. Audio pipeline

```
mic → resample to 16 kHz mono → gain / AGC → level meter → encoder → per-listener gate → session queue
session → jitter buffer (per station) → decoder / concealment → volume → mixer → speaker
```

- Internal capture format: 16 kHz, mono, 16-bit signed PCM, frames of 20 ms
  (320 samples). Implementations resample from the device rate.
- **Level**: the RMS of each 20 ms frame after gain, in dBFS
  (`20·log10(rms / 32768)`, floor `-90`). The value used for meters, `level`
  records and sound activation is this per-frame level.

### 1.1 Gain and automatic gain control

- Manual gain steps: 0, +6, +12, +18 dB.
- AGC (default on) targets a speech-like level of `-24 dBFS` for loud
  events. It raises gain slowly (at most +3 dB per second, up to +24 dB) and
  lowers it fast (attack within 50 ms), never amplifying frames below
  `-70 dBFS` so the background noise floor is not pumped up. AGC runs before
  the level meter, so thresholds refer to what listeners hear.

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
