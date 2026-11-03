// SPDX-License-Identifier: AGPL-3.0-or-later
// Audio worklets of the Lull browser listener (platforms.md §2.5).
//
// lull-playback: plays 16 kHz mono frames that the main thread pulls from
// the listener engine (pull_playback). The worklet asks for frames whenever
// less than TARGET samples are buffered or requested, so the engine is
// pulled at the audio device's pace (CORE-12).
//
// lull-capture: posts microphone blocks for talk-back (push_talk_capture).

const FRAME = 320; // 20 ms at 16 kHz
const TARGET = 1600; // 100 ms buffered or in flight

class LullPlayback extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.buffered = 0;
    this.requested = 0;
    this.underruns = 0;
    this.played = 0;
    this.port.onmessage = (e) => {
      const f = e.data;
      if (f instanceof Float32Array) {
        this.queue.push(f);
        this.buffered += f.length;
        this.requested = Math.max(0, this.requested - f.length);
      }
    };
    this.request();
  }

  request() {
    const missing = TARGET - this.buffered - this.requested;
    if (missing > 0) {
      const frames = Math.ceil(missing / FRAME);
      this.requested += frames * FRAME;
      this.port.postMessage({ need: frames, underruns: this.underruns, played: this.played });
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0][0];
    let i = 0;
    while (i < out.length && this.queue.length > 0) {
      const f = this.queue[0];
      const n = Math.min(out.length - i, f.length - this.offset);
      out.set(f.subarray(this.offset, this.offset + n), i);
      i += n;
      this.offset += n;
      this.buffered -= n;
      if (this.offset >= f.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    if (i < out.length) {
      out.fill(0, i);
      this.underruns++;
    }
    this.played += out.length;
    this.request();
    return true;
  }
}

class LullCapture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buf = new Float32Array(1024);
    this.n = 0;
  }

  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch) {
      for (let i = 0; i < ch.length; i++) {
        this.buf[this.n++] = ch[i];
        if (this.n === this.buf.length) {
          this.port.postMessage(this.buf, [this.buf.buffer]);
          this.buf = new Float32Array(1024);
          this.n = 0;
        }
      }
    }
    return true;
  }
}

registerProcessor("lull-playback", LullPlayback);
registerProcessor("lull-capture", LullCapture);
