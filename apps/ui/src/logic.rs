// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pure presentation logic: level mapping, formatting, status pills and
//! calibration chart geometry. No Dioxus, no engines; unit tested.

use crate::model::{Conn, ListenerRow, Mode, NetPath, Talk, Tile};
use crate::strings as s;

/// dBFS shown at the left end of meters (0 %).
pub const METER_FLOOR_DBFS: f32 = -90.0;
/// dBFS shown at the right end of meters (100 %).
pub const METER_CEIL_DBFS: f32 = -10.0;
/// Lowest threshold the user can choose.
pub const THRESHOLD_MIN_DBFS: f32 = -85.0;
/// Highest threshold the user can choose.
pub const THRESHOLD_MAX_DBFS: f32 = -10.0;

/// Maps a level to the 0..1 position used by `--level` / `--threshold`
/// (linear in dB between [`METER_FLOOR_DBFS`] and [`METER_CEIL_DBFS`]).
pub fn level_fraction(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs - METER_FLOOR_DBFS) / (METER_CEIL_DBFS - METER_FLOOR_DBFS)).clamp(0.0, 1.0)
}

/// Inverse of [`level_fraction`].
pub fn fraction_to_dbfs(f: f32) -> f32 {
    let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) };
    METER_FLOOR_DBFS + f * (METER_CEIL_DBFS - METER_FLOOR_DBFS)
}

/// Clamps and rounds a threshold to whole dB in the selectable range.
pub fn clamp_threshold(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return -50.0;
    }
    dbfs.round().clamp(THRESHOLD_MIN_DBFS, THRESHOLD_MAX_DBFS)
}

/// A CSS number for custom properties (`0.512`).
pub fn css_fraction(f: f32) -> String {
    format!("{:.3}", f.clamp(0.0, 1.0))
}

/// `−56` with a true minus sign.
pub fn db_number(dbfs: f32) -> String {
    let v = dbfs.round() as i32;
    if v < 0 {
        format!("\u{2212}{}", -v)
    } else {
        v.to_string()
    }
}

/// `−56 dB`.
pub fn db(dbfs: f32) -> String {
    format!("{} {}", db_number(dbfs), s::DB)
}

/// `0:42`, `9:12`, `1:02:03`.
pub fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, sec) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// Remaining pairing time, rounded up to the second (`9:12`).
pub fn remaining(ms: u64) -> String {
    duration(ms.div_ceil(1000) * 1000)
}

/// "5 min ago" style for paired/last seen dates.
pub fn ago(then_unix_ms: u64, now_unix_ms: u64) -> String {
    let d = now_unix_ms.saturating_sub(then_unix_ms) / 1000;
    match d {
        0..60 => s::AGO_NOW.into(),
        60..3_600 => s::ago_minutes(d / 60),
        3_600..86_400 => s::ago_hours(d / 3_600),
        _ => s::ago_days(d / 86_400),
    }
}

/// A device ID in groups of four (`abcd-efgh-ijkl-mnop`, protocol.md §2.2).
pub fn grouped_id(id: &str) -> String {
    let clean: Vec<char> = id.chars().filter(|c| *c != '-').collect();
    clean
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Formats pairing code input as the user types: Crockford characters
/// only, upper case, at most 10, with the hyphen after five
/// (`7KQ2M-X9RTA`, protocol.md §5.3). Ambiguous letters are mapped like the
/// engine does (`O`→`0`, `I`/`L`→`1`).
pub fn format_code_input(input: &str) -> String {
    let mut chars: Vec<char> = Vec::with_capacity(10);
    for c in input.chars() {
        let c = c.to_ascii_uppercase();
        let c = match c {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if c.is_ascii_digit() || (c.is_ascii_uppercase() && c != 'U') {
            chars.push(c);
        }
        if chars.len() == 10 {
            break;
        }
    }
    let mut out: String = chars.iter().take(5).collect();
    if chars.len() > 5 {
        out.push('-');
        out.extend(chars.iter().skip(5));
    }
    out
}

/// Whether a formatted code is complete.
pub fn code_complete(code: &str) -> bool {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).count() == 10
}

/// Class and text of a status pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    /// Modifier class (`calm`, `sound`, `warn`, `voice`, `cam`, or empty).
    pub class: &'static str,
    /// Words, never colour alone (NFR-A11Y-1).
    pub text: String,
}

impl Pill {
    fn new(class: &'static str, text: impl Into<String>) -> Self {
        Self {
            class,
            text: text.into(),
        }
    }

    /// `pill calm` etc.
    pub fn classes(&self) -> String {
        if self.class.is_empty() {
            "pill".into()
        } else {
            format!("pill {}", self.class)
        }
    }
}

/// Whether the room is above this listener's threshold right now.
pub fn is_loud(tile: &Tile) -> bool {
    tile.level_dbfs
        .is_some_and(|l| l >= tile.settings.threshold_dbfs)
}

/// Whether the tile shows "Sound" (and the coral halo).
pub fn has_sound(tile: &Tile) -> bool {
    tile.conn == Conn::Live
        && match tile.settings.mode {
            Mode::Activated => tile.open,
            Mode::Continuous => is_loud(tile),
        }
}

/// The status pill of a tile (FR-LS-3).
pub fn tile_pill(tile: &Tile) -> Pill {
    match &tile.conn {
        Conn::Live => {
            if tile.talk == Talk::Granted {
                Pill::new("voice", s::STATUS_YOU_TALK)
            } else if let Some(t) = &tile.talker {
                Pill::new("voice", s::status_talking(Some(t)))
            } else if has_sound(tile) {
                Pill::new("sound", s::STATUS_SOUND)
            } else {
                Pill::new("calm", s::STATUS_QUIET)
            }
        }
        Conn::Connecting { .. } => Pill::new("warn", s::STATUS_CONNECTING),
        Conn::Reconnecting { .. } => Pill::new("warn", s::STATUS_RECONNECTING),
        Conn::NeedsPairing => Pill::new("warn", s::STATUS_NEEDS_PAIRING),
        Conn::Paused => Pill::new("", s::STATUS_PAUSED),
    }
}

/// The caption under a tile's name: path or reconnect progress (FR-LS-5).
pub fn tile_subtitle(tile: &Tile) -> String {
    match &tile.conn {
        Conn::Live => match tile.path {
            Some(NetPath::Vpn) => s::PATH_VPN.into(),
            Some(NetPath::Lan) => s::PATH_LAN.into(),
            None => String::new(),
        },
        Conn::Connecting { tried, .. } | Conn::Reconnecting { tried, .. } => {
            s::trying_addresses((*tried).max(tile.routes.min(16)))
        }
        Conn::NeedsPairing | Conn::Paused => String::new(),
    }
}

/// "−18 dB now · opens above −50 dB".
pub fn level_caption(tile: &Tile) -> String {
    let Some(level) = tile.level_dbfs else {
        return s::NO_LEVEL.into();
    };
    let level = if tile.open {
        s::level_now(&db(level))
    } else {
        s::level_room(&db(level))
    };
    match tile.settings.mode {
        Mode::Activated => format!(
            "{level} · {}",
            s::opens_above(&db(tile.settings.threshold_dbfs))
        ),
        Mode::Continuous => format!("{level} · {}", s::ALWAYS_ON_CAPTION),
    }
}

/// Battery caption (FR-LS-3), `None` without battery data.
pub fn battery_caption(battery: Option<u8>, charging: Option<bool>) -> Option<String> {
    match (battery, charging) {
        (Some(p), Some(true)) => Some(s::battery_charging(p)),
        (Some(p), _) => Some(s::battery(p)),
        (None, Some(true)) => Some(s::CHARGING.into()),
        _ => None,
    }
}

/// Whether the battery is low enough to show the warning colour.
pub fn battery_low(battery: Option<u8>, charging: Option<bool>) -> bool {
    battery.is_some_and(|b| b < 15) && charging != Some(true)
}

/// The ring tick on the station: the lowest (most sensitive) threshold of
/// the connected sound-activated listeners, above which at least one of
/// them hears the room (OPEN-QUESTIONS UI-2).
pub fn strictest_threshold(listeners: &[ListenerRow]) -> Option<f32> {
    listeners
        .iter()
        .filter(|l| l.mode == Mode::Activated)
        .map(|l| l.threshold_dbfs)
        .reduce(f32::min)
}

/// Whether sound activation is open for any sound-activated listener
/// (continuous listeners always receive audio, so they do not count).
pub fn anyone_hearing(listeners: &[ListenerRow]) -> bool {
    listeners
        .iter()
        .any(|l| l.open && l.mode == Mode::Activated)
}

/// Chart width in SVG user units.
pub const CHART_W: f32 = 300.0;
/// Chart height in SVG user units (also the CSS height of `.calibrate`).
pub const CHART_H: f32 = 120.0;

/// SVG path of the last minute of levels: x is time (right edge = `now`),
/// y the level fraction (top = loud). Points older than the window are
/// skipped; an empty history gives an empty path.
pub fn chart_path(points: impl IntoIterator<Item = (u64, f32)>, now_ms: u64) -> String {
    let window = crate::model::LevelHistory::WINDOW_MS as f32;
    let mut d = String::new();
    for (t, dbfs) in points {
        let age = now_ms.saturating_sub(t) as f32;
        if age > window {
            continue;
        }
        let x = CHART_W * (1.0 - age / window);
        let y = CHART_H * (1.0 - level_fraction(dbfs));
        let cmd = if d.is_empty() { 'M' } else { 'L' };
        if !d.is_empty() {
            d.push(' ');
        }
        d.push_str(&format!("{cmd}{x:.1} {y:.1}"));
    }
    d
}

/// Top of the threshold line in percent of the chart height.
pub fn threshold_top_percent(dbfs: f32) -> f32 {
    (1.0 - level_fraction(dbfs)) * 100.0
}

/// The threshold for a pointer at `y` pixels from the top of a chart
/// `height` pixels high (dragging the line).
pub fn threshold_at(y: f64, height: f64) -> f32 {
    if height <= 0.0 || y.is_nan() {
        return -50.0;
    }
    let f = 1.0 - (y / height).clamp(0.0, 1.0);
    clamp_threshold(fraction_to_dbfs(f as f32))
}

/// Keyboard steps of the threshold slider.
pub fn threshold_step(dbfs: f32, key: ThresholdKey) -> f32 {
    let v = match key {
        ThresholdKey::Up => dbfs + 1.0,
        ThresholdKey::Down => dbfs - 1.0,
        ThresholdKey::PageUp => dbfs + 5.0,
        ThresholdKey::PageDown => dbfs - 5.0,
        ThresholdKey::Home => THRESHOLD_MIN_DBFS,
        ThresholdKey::End => THRESHOLD_MAX_DBFS,
    };
    clamp_threshold(v)
}

/// Keys of the threshold slider (WAI-ARIA slider pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdKey {
    /// Arrow up / right.
    Up,
    /// Arrow down / left.
    Down,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Home: minimum.
    Home,
    /// End: maximum.
    End,
}

/// Hold time slider in seconds (1..15) from milliseconds.
pub fn hold_seconds(ms: u32) -> u32 {
    ms.div_ceil(1000).clamp(1, 15)
}

/// Volume percent from the linear volume.
pub fn volume_percent(v: f32) -> u32 {
    if v.is_nan() {
        0
    } else {
        (v.clamp(0.0, 1.0) * 100.0).round() as u32
    }
}

/// The text of the station's mode chip for one listener row.
pub fn listener_row_caption(l: &ListenerRow) -> String {
    let mut parts: Vec<&str> = vec![match l.mode {
        Mode::Activated => s::MODE_SOUND_ACTIVATED,
        Mode::Continuous => s::MODE_ALWAYS_ON,
    }];
    if l.open {
        parts.push(s::HEARING);
    }
    if l.talking {
        parts.push(s::TALKING_NOW);
    }
    if l.watching {
        parts.push(s::WATCHING_NOW);
    }
    if l.web {
        parts.push(s::VIA_BROWSER);
    }
    parts.join(" · ")
}

/// "Camera on · Mum": who watches.
pub fn camera_pill(listeners: &[ListenerRow]) -> String {
    let watchers: Vec<&str> = listeners
        .iter()
        .filter(|l| l.watching)
        .map(|l| l.name.as_str())
        .collect();
    if watchers.is_empty() {
        s::CAMERA_ON.into()
    } else {
        format!("{} · {}", s::CAMERA_ON, watchers.join(", "))
    }
}

/// Station pill: "2 listening".
pub fn listening_count(n: usize) -> String {
    s::listening_count(n)
}

/// Minimal percent-encoding for a URL fragment (RFC 3986 unreserved kept),
/// used to build web links (protocol.md §3.5, PROTO-10).
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TileSettings, Video};

    fn tile() -> Tile {
        Tile {
            id: "a".into(),
            name: "Nursery".into(),
            conn: Conn::Live,
            path: Some(NetPath::Lan),
            route: None,
            level_dbfs: Some(-58.0),
            open: false,
            battery: Some(82),
            charging: Some(false),
            station_listeners: Some(1),
            camera_on: false,
            talker: None,
            talk: Talk::Idle,
            video: Video::Off,
            can_talk: true,
            can_video: true,
            settings: TileSettings::default(),
            down_for_ms: None,
            routes: 2,
            alerting: false,
        }
    }

    #[test]
    fn level_mapping_is_linear_clamped_and_invertible() {
        assert_eq!(level_fraction(-90.0), 0.0);
        assert_eq!(level_fraction(-10.0), 1.0);
        assert_eq!(level_fraction(-50.0), 0.5);
        assert_eq!(level_fraction(5.0), 1.0);
        assert_eq!(level_fraction(-200.0), 0.0);
        assert_eq!(level_fraction(f32::NAN), 0.0);
        for db in [-85.0, -50.0, -23.0] {
            assert!((fraction_to_dbfs(level_fraction(db)) - db).abs() < 1e-3);
        }
        assert_eq!(css_fraction(0.5), "0.500");
    }

    #[test]
    fn db_and_time_formatting() {
        assert_eq!(db(-18.4), "\u{2212}18 dB");
        assert_eq!(db_number(0.2), "0");
        assert_eq!(duration(42_000), "0:42");
        assert_eq!(duration(3_723_000), "1:02:03");
        assert_eq!(remaining(551_200), "9:12");
        assert_eq!(remaining(0), "0:00");
        assert_eq!(ago(0, 30_000), s::AGO_NOW);
        assert_eq!(ago(0, 5 * 60_000), s::ago_minutes(5));
        assert_eq!(ago(0, 3 * 86_400_000), s::ago_days(3));
    }

    #[test]
    fn code_input_is_normalised_and_grouped() {
        assert_eq!(format_code_input("7kq2mx9rta"), "7KQ2M-X9RTA");
        assert_eq!(format_code_input("7KQ2M-X9RTA-EXTRA"), "7KQ2M-X9RTA");
        assert_eq!(format_code_input("oil u"), "011");
        assert!(code_complete("7KQ2M-X9RTA"));
        assert!(!code_complete("7KQ2M"));
        assert_eq!(grouped_id("abcdefghijklmnop"), "abcd-efgh-ijkl-mnop");
    }

    #[test]
    fn pills_carry_words() {
        let mut t = tile();
        assert_eq!(tile_pill(&t), Pill::new("calm", s::STATUS_QUIET));
        t.open = true;
        assert_eq!(tile_pill(&t).class, "sound");
        assert!(has_sound(&t));
        t.talker = Some("Dad".into());
        assert_eq!(tile_pill(&t).text, "Dad talking");
        t.talk = Talk::Granted;
        assert_eq!(tile_pill(&t).text, s::STATUS_YOU_TALK);
        t.conn = Conn::Reconnecting {
            attempt: 2,
            tried: 3,
        };
        assert_eq!(tile_pill(&t), Pill::new("warn", s::STATUS_RECONNECTING));
        assert_eq!(tile_subtitle(&t), "trying 3 addresses…");
        assert!(!has_sound(&t));
        t.conn = Conn::Paused;
        assert_eq!(tile_pill(&t).classes(), "pill");
    }

    #[test]
    fn continuous_mode_uses_the_level_for_sound() {
        let mut t = tile();
        t.settings.mode = Mode::Continuous;
        t.open = true;
        assert!(!has_sound(&t));
        t.level_dbfs = Some(-20.0);
        assert!(has_sound(&t));
        assert_eq!(
            level_caption(&t),
            format!(
                "{} · {}",
                s::level_now("\u{2212}20 dB"),
                s::ALWAYS_ON_CAPTION
            )
        );
    }

    #[test]
    fn captions() {
        let t = tile();
        assert_eq!(
            level_caption(&t),
            "\u{2212}58 dB room · opens above \u{2212}50 dB"
        );
        assert_eq!(tile_subtitle(&t), s::PATH_LAN);
        assert_eq!(battery_caption(Some(82), Some(false)), Some(s::battery(82)));
        assert_eq!(battery_caption(None, None), None);
        assert!(battery_low(Some(10), None));
        assert!(!battery_low(Some(10), Some(true)));
    }

    fn row(mode: Mode, th: f32, watching: bool) -> ListenerRow {
        ListenerRow {
            session: 1,
            id: "x".into(),
            name: "Mum".into(),
            mode,
            threshold_dbfs: th,
            open: false,
            talking: false,
            watching,
            web: false,
        }
    }

    #[test]
    fn strictest_threshold_ignores_continuous_listeners() {
        assert_eq!(strictest_threshold(&[]), None);
        let rows = [
            row(Mode::Activated, -40.0, false),
            row(Mode::Continuous, -90.0, true),
            row(Mode::Activated, -55.0, false),
        ];
        assert_eq!(strictest_threshold(&rows), Some(-55.0));
        let mut open = rows.clone();
        open[1].open = true;
        assert!(!anyone_hearing(&open), "continuous listeners always hear");
        open[0].open = true;
        assert!(anyone_hearing(&open));
        assert_eq!(camera_pill(&rows), format!("{} · Mum", s::CAMERA_ON));
        assert_eq!(listener_row_caption(&rows[1]), "always on · camera");
    }

    #[test]
    fn chart_geometry() {
        let now = 100_000;
        let pts = vec![(30_000, -50.0), (40_000, -90.0), (100_000, -10.0)];
        // The first point is older than a minute and skipped.
        assert_eq!(chart_path(pts, now), "M0.0 120.0 L300.0 0.0");
        assert_eq!(chart_path(Vec::new(), now), "");
        assert_eq!(threshold_top_percent(-50.0), 50.0);
        assert_eq!(threshold_at(60.0, 120.0), -50.0);
        assert_eq!(threshold_at(0.0, 120.0), THRESHOLD_MAX_DBFS);
        assert_eq!(threshold_at(500.0, 120.0), THRESHOLD_MIN_DBFS);
        assert_eq!(threshold_at(10.0, 0.0), -50.0);
        assert_eq!(threshold_step(-50.0, ThresholdKey::PageUp), -45.0);
        assert_eq!(threshold_step(-12.0, ThresholdKey::PageUp), -10.0);
        assert_eq!(threshold_step(-50.0, ThresholdKey::Home), -85.0);
    }

    #[test]
    fn sliders_and_encoding() {
        assert_eq!(hold_seconds(4_000), 4);
        assert_eq!(hold_seconds(0), 1);
        assert_eq!(hold_seconds(60_000), 15);
        assert_eq!(volume_percent(0.704), 70);
        assert_eq!(percent_encode("nestwatch://a b"), "nestwatch%3A%2F%2Fa%20b");
    }
}
