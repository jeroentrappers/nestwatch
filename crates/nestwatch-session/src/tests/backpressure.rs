// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scheduling and backpressure (protocol.md §8, media.md §3, §5).

use nestwatch_proto::control::Control;
use nestwatch_proto::record::{AudioFlags, AudioStream, VIDEO_CODEC_JPEG};
use nestwatch_proto::video::VideoFrame;

use super::harness::*;
use super::records::audio;
use crate::{
    AUDIO_QUEUE_FRAMES, Flavor, ListenerEvent, SendError, SessionError, StationConfig, StationEvent,
};

fn jpeg(frame_id: u32, len: usize) -> VideoFrame {
    VideoFrame {
        codec: VIDEO_CODEC_JPEG,
        frame_id,
        capture_ms: frame_id * 100,
        width: 640,
        height: 480,
        data: (0..len).map(|i| (i as u32 ^ frame_id) as u8).collect(),
    }
}

fn net_with_fragment(flavor: Flavor, fragment: usize) -> Net {
    let mut config = StationConfig::new(flavor, station_hello());
    config.video_fragment_len = fragment;
    let s = station_with(config, TestPolicy::trusting(), 0);
    let mut net = Net::with(flavor, listener(flavor, trusted_mode(), 0), s);
    net.connect();
    net.l_events.clear();
    net.s_events.clear();
    net
}

fn videos(events: &[ListenerEvent]) -> Vec<VideoFrame> {
    events
        .iter()
        .filter_map(|e| match e {
            ListenerEvent::Video(f) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn video_frames_reassembled_section_6_2() {
    for flavor in [Flavor::Tcp, Flavor::WebSocket] {
        let mut net = net_with_fragment(flavor, crate::DEFAULT_VIDEO_FRAGMENT_LEN);
        for id in 1..=3 {
            net.s.send_video(&jpeg(id, 50_000 + id as usize)).unwrap();
            net.run_for(5);
        }
        assert_eq!(
            videos(&net.l_events),
            vec![jpeg(1, 50_001), jpeg(2, 50_002), jpeg(3, 50_003)]
        );
    }
}

#[test]
fn priority_control_then_audio_then_video_section_8() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.chunk = 100;
    net.writable = false;
    net.s.send_video(&jpeg(1, 2_500)).unwrap();
    for seq in 0..3 {
        net.s
            .send_audio(audio(AudioStream::Station, seq), &[0; 160])
            .unwrap();
    }
    net.s
        .send_control(Control::Activity { open: true })
        .unwrap();
    net.writable = true;
    net.run_for(5);
    let order: Vec<&str> = net
        .l_events
        .iter()
        .map(|e| match e {
            ListenerEvent::Control(_) => "control",
            ListenerEvent::Audio { .. } => "audio",
            ListenerEvent::Video(_) => "video",
            _ => "other",
        })
        .collect();
    assert_eq!(order, ["control", "audio", "audio", "audio", "video"]);
}

#[test]
fn control_overtakes_queued_video_fragments_section_8() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    net.s.send_video(&jpeg(1, 10_000)).unwrap();
    // The socket takes one fragment, then a control message is queued.
    assert!(net.pull_station_once(1) > 0);
    net.s
        .send_control(Control::Activity { open: false })
        .unwrap();
    net.writable = true;
    net.run_for(5);
    assert!(matches!(net.l_events[0], ListenerEvent::Control(_)));
    assert_eq!(videos(&net.l_events), vec![jpeg(1, 10_000)]);
}

#[test]
fn audio_queue_keeps_newest_500_ms_section_8() {
    let mut net = net_with_fragment(Flavor::WebSocket, 1_000);
    net.writable = false;
    for seq in 0..40 {
        net.s
            .send_audio(audio(AudioStream::Station, seq), &[0; 160])
            .unwrap();
    }
    assert_eq!(AUDIO_QUEUE_FRAMES, 25);
    assert_eq!(net.s.stats().audio_dropped, 15);
    net.writable = true;
    net.run_for(5);
    assert_eq!(net.l_audio_seqs(), (15..40).collect::<Vec<_>>());
}

#[test]
fn preroll_frames_dropped_first_media_3() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    for seq in 0..30 {
        let mut h = audio(AudioStream::Station, seq);
        h.flags = AudioFlags::new(seq < 5, seq == 5);
        net.s.send_audio(h, &[0; 160]).unwrap();
    }
    net.writable = true;
    net.run_for(5);
    // The 5 pre-roll frames went first, the live burst survived intact.
    assert_eq!(net.l_audio_seqs(), (5..30).collect::<Vec<_>>());
}

#[test]
fn queued_video_frame_is_replaced_and_reported_section_8_media_5() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    net.s.send_video(&jpeg(1, 3_000)).unwrap();
    net.s.send_video(&jpeg(2, 3_000)).unwrap();
    net.s.send_video(&jpeg(3, 3_000)).unwrap();
    net.writable = true;
    net.run_for(5);
    assert_eq!(
        net.s_events,
        vec![
            StationEvent::VideoFrameReplaced { frame_id: 1 },
            StationEvent::VideoFrameReplaced { frame_id: 2 },
        ]
    );
    assert_eq!(net.s.stats().video_replaced, 2);
    assert_eq!(videos(&net.l_events), vec![jpeg(3, 3_000)]);
}

#[test]
fn partly_sent_video_frame_finishes_and_next_waits_section_8_session_5() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    net.s.send_video(&jpeg(1, 3_000)).unwrap();
    // The first of three fragments leaves: frame 1 is committed.
    assert!(net.pull_station_once(500) > 0);
    // Frame 2 waits behind it; frame 3 replaces the waiting frame 2.
    net.s.send_video(&jpeg(2, 3_000)).unwrap();
    assert_eq!(net.s.poll_event(), None);
    net.s.send_video(&jpeg(3, 3_000)).unwrap();
    assert_eq!(
        net.s.poll_event(),
        Some(StationEvent::VideoFrameReplaced { frame_id: 2 })
    );
    net.writable = true;
    net.run_for(5);
    assert_eq!(videos(&net.l_events), vec![jpeg(1, 3_000), jpeg(3, 3_000)]);
}

#[test]
fn fully_sent_video_frame_is_not_reported_as_replaced() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.s.send_video(&jpeg(1, 3_000)).unwrap();
    net.run_for(5);
    net.s.send_video(&jpeg(2, 3_000)).unwrap();
    net.run_for(5);
    assert!(net.s_events.is_empty());
    assert_eq!(videos(&net.l_events).len(), 2);
}

#[test]
fn video_needs_a_codec_the_listener_accepts() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    let mut f = jpeg(1, 10);
    f.codec = 9;
    assert_eq!(
        net.s.send_video(&f),
        Err(SendError::VideoCodecNotAccepted(9))
    );
    f.codec = VIDEO_CODEC_JPEG;
    f.data.clear();
    assert!(matches!(net.s.send_video(&f), Err(SendError::Video(_))));
}

#[test]
fn control_queue_stalled_for_10_s_closes_section_8() {
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    net.s
        .send_control(Control::Activity { open: true })
        .unwrap();
    net.run_for(9_999);
    assert!(net.s_failed().is_none());
    net.run_for(1);
    assert_eq!(net.s_failed(), Some(&SessionError::ControlStalled));
    assert!(net.s.is_finished());
}

#[test]
fn slow_socket_keeps_control_and_audio_flowing_section_8() {
    // A socket taking 300 bytes per ms (2.4 Mbit/s) while the camera offers
    // 20 kB frames every 20 ms (8 Mbit/s): control and audio all arrive,
    // video degrades by replacement but complete frames still get through.
    let mut net = net_with_fragment(Flavor::Tcp, 1_000);
    net.writable = false;
    for i in 0..50u32 {
        net.s.send_video(&jpeg(i, 20_000)).unwrap();
        net.s
            .send_audio(audio(AudioStream::Station, i), &[0; 160])
            .unwrap();
        if i % 10 == 0 {
            net.s
                .send_control(Control::Activity { open: i % 20 == 0 })
                .unwrap();
        }
        for _ in 0..20 {
            net.pull_station_once(300);
            net.run_for(1);
        }
    }
    let controls = net
        .l_events
        .iter()
        .filter(|e| matches!(e, ListenerEvent::Control(_)))
        .count();
    assert_eq!(controls, 5);
    assert_eq!(net.l_audio_seqs(), (0..50).collect::<Vec<_>>());
    let shown = videos(&net.l_events).len();
    assert!(shown >= 5, "{shown} frames shown");
    assert!(net.s.stats().video_replaced >= 20);
    assert!(net.s_failed().is_none());
}

#[test]
fn poll_transmit_respects_max_and_websocket_sends_whole_messages_section_8() {
    let mut net = net_with_fragment(Flavor::Tcp, 4_000);
    net.writable = false;
    net.s.send_video(&jpeg(1, 30_000)).unwrap();
    let mut total = 0;
    while let Some(b) = net.s.poll_transmit(net.now, 777) {
        assert!(b.len() <= 777);
        total += b.len();
    }
    assert!(total > 30_000);
    assert_eq!(net.s.poll_transmit(net.now, 0), None);

    let mut net = net_with_fragment(Flavor::WebSocket, 4_000);
    net.writable = false;
    net.s.send_video(&jpeg(1, 30_000)).unwrap();
    let mut messages = 0;
    while let Some(m) = net.s.poll_transmit(net.now, 1) {
        // 1 type byte + 17 header + fragment + 16 tag.
        assert!(m.len() <= 4_000 + 34);
        messages += 1;
    }
    assert_eq!(messages, 8);
}
