// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every user-facing English string of the Dioxus apps (product.md
//! NFR-I18N-1). Components never contain literal text; a translation
//! replaces this module (or loads a table with the same names).
//!
//! Voice (design README §2): short, kind, specific.

/// Product name.
pub const APP: &str = "Nestwatch";
/// Promise on the welcome screen.
pub const PROMISE: &str =
    "A private baby monitor for the devices you already own. Everything stays on your network.";
/// Medical disclaimer (product.md §1), first launch and About.
pub const DISCLAIMER: &str = "Nestwatch is not a medical device and does not replace supervision. Always check on your child in person.";
/// Accept the disclaimer.
pub const DISCLAIMER_ACCEPT: &str = "I understand";
/// Role choice: station.
pub const ROLE_STATION: &str = "This device watches";
/// Explanation of the station role.
pub const ROLE_STATION_HINT: &str =
    "Put it in the child's room. It listens and, when asked, shows the camera.";
/// Role choice: listener.
pub const ROLE_LISTENER: &str = "This device listens";
/// Explanation of the listener role.
pub const ROLE_LISTENER_HINT: &str = "Follow one or more rooms from here.";
/// Role switch group label.
pub const ROLE_GROUP: &str = "Role";
/// Role switch: listener.
pub const ROLE_TAB_LISTENING: &str = "Listening";
/// Role switch: station.
pub const ROLE_TAB_STATION: &str = "Station";
/// Browser cannot be a station.
pub const ROLE_STATION_UNAVAILABLE: &str =
    "A browser can only listen. Use the Nestwatch app on a phone or computer as the station.";

// ----- common ---------------------------------------------------------------

/// Generic close.
pub const CLOSE: &str = "Close";
/// Back.
pub const BACK: &str = "Back";
/// Done.
pub const DONE: &str = "Done";
/// Cancel.
pub const CANCEL: &str = "Cancel";
/// Save.
pub const SAVE: &str = "Save";
/// Settings button label.
pub const SETTINGS: &str = "Settings";
/// About.
pub const ABOUT: &str = "About Nestwatch";
/// Copy.
pub const COPY: &str = "Copy link";
/// Copied.
pub const COPIED: &str = "Copied";
/// Theme group label.
pub const THEME: &str = "Theme";
/// Theme: follow the system.
pub const THEME_SYSTEM: &str = "System";
/// Theme: night.
pub const THEME_NIGHT: &str = "Night";
/// Theme: day.
pub const THEME_DAY: &str = "Day";
/// Dismiss a notice.
pub const DISMISS: &str = "Dismiss";
/// Alerts region label.
pub const NOTICES: &str = "Alerts";
/// Unknown name placeholder.
pub const UNNAMED: &str = "Unnamed device";
/// About text.
pub const ABOUT_BODY: &str = "Nestwatch connects your own devices directly, on your home network or your own VPN. There is no account, no cloud and no tracking. Audio and video are end-to-end encrypted.";
/// About: version line prefix.
pub const VERSION: &str = "Version";
/// About: device ID label.
pub const DEVICE_ID: &str = "Device ID";
/// About: licence.
pub const LICENSE: &str = "Free software under the GNU AGPL 3.0 or later.";

// ----- station --------------------------------------------------------------

/// Station is starting.
pub const STATION_STARTING: &str = "Starting the station…";
/// Station could not start.
pub const STATION_FAILED: &str = "The station could not start";
/// Retry.
pub const RETRY: &str = "Try again";
/// Station not running.
pub const STATION_STOPPED: &str = "The station is not running.";
/// Start the station.
pub const STATION_START: &str = "Start the station";
/// Stop the station.
pub const STATION_STOP: &str = "Stop the station";
/// Pair a listener.
pub const PAIR_LISTENER: &str = "Pair a listener";
/// Dim screen.
pub const DIM_SCREEN: &str = "Dim screen";
/// Tap to brighten.
pub const TAP_TO_BRIGHTEN: &str = "Tap anywhere to brighten";
/// Station screen is dimmed (screen reader).
pub const DIMMED_LABEL: &str = "Screen dimmed. Monitoring continues. Activate to brighten.";
/// No listeners yet.
pub const NO_LISTENERS: &str = "Nobody is listening yet.";
/// Connected listeners heading (screen reader).
pub const LISTENERS_HEADING: &str = "Listening now";
/// Mode: sound activated.
pub const MODE_SOUND_ACTIVATED: &str = "sound-activated";
/// Mode: always on.
pub const MODE_ALWAYS_ON: &str = "always on";
/// Listener uses the browser.
pub const VIA_BROWSER: &str = "browser";
/// Hearing now.
pub const HEARING: &str = "hearing the room";
/// Talking now.
pub const TALKING_NOW: &str = "talking";
/// Watching now.
pub const WATCHING_NOW: &str = "camera";
/// Ring caption: quiet.
pub const RING_QUIET: &str = "quiet";
/// Ring caption: sound.
pub const RING_SOUND: &str = "sound";
/// Ring unit.
pub const DB: &str = "dB";
/// Ring label for screen readers.
pub const RING_LABEL: &str = "Room level";
/// Camera on indicator, nobody named.
pub const CAMERA_ON: &str = "Camera on";
/// Camera off indicator.
pub const CAMERA_OFF: &str = "Camera off";
/// Talk-back off.
pub const TALK_OFF: &str = "Talk-back off";
/// Station background warning (FR-ST-5).
pub const STATION_KEEPS_RUNNING: &str = "Keep this app open. Closing the window keeps the station running in the tray; quitting stops monitoring.";
/// Station settings heading.
pub const STATION_SETTINGS: &str = "Station settings";
/// Station name field.
pub const STATION_NAME: &str = "Station name";
/// Camera access switch.
pub const CAMERA_ALLOWED: &str = "Allow the camera";
/// Talk-back switch.
pub const TALK_ALLOWED: &str = "Allow talk-back";
/// Gain field.
pub const MIC_SENSITIVITY: &str = "Microphone sensitivity";
/// Gain: automatic.
pub const GAIN_AUTO: &str = "Auto";
/// Talk-back volume.
pub const TALKBACK_VOLUME: &str = "Talk-back volume";
/// Trusted listeners heading.
pub const TRUSTED_LISTENERS: &str = "Paired listeners";
/// No trusted listeners.
pub const NO_TRUSTED: &str = "No listener is paired yet.";
/// Revoke.
pub const REVOKE: &str = "Remove";
/// HTTPS fingerprint heading.
pub const HTTPS_FINGERPRINT: &str = "HTTPS certificate fingerprint";
/// Fingerprint explanation.
pub const HTTPS_FINGERPRINT_HINT: &str =
    "Browsers warn about the station's own certificate. Accept it only if the fingerprint matches.";
/// Browser address heading.
pub const BROWSER_ADDRESS: &str = "Browser address";
/// Camera unavailable on this station.
pub const CAMERA_UNAVAILABLE_HERE: &str = "No camera found";
/// Camera permission denied on this station.
pub const CAMERA_DENIED_HERE: &str = "Camera permission denied";

// ----- pairing sheet (station) ------------------------------------------------

/// Pairing sheet intro.
pub const PAIR_INTRO: &str = "Scan with the Nestwatch app, or open the link on a computer.";
/// Code intro.
pub const PAIR_CODE_INTRO: &str = "No camera? Enter this code";
/// Station ID caption on the sheet (for code pairing in a browser).
pub const PAIR_STATION_ID: &str = "Station ID";
/// QR code label.
pub const QR_LABEL: &str = "Pairing QR code";
/// Pairing is opening.
pub const PAIR_OPENING: &str = "Opening pairing…";
/// Pairing ended.
pub const PAIR_CLOSED: &str = "Pairing is closed.";
/// Paired listeners heading on the sheet.
pub const PAIR_JUST_PAIRED: &str = "Paired now. Check that the listener shows the same number:";
/// Link label.
pub const PAIR_LINK: &str = "Web link";

// ----- add a station (listener) ----------------------------------------------

/// Add a station.
pub const ADD_STATION: &str = "Add a station";
/// Add station button.
pub const ADD_STATION_BUTTON: &str = "+ Add station";
/// Nearby heading.
pub const NEARBY: &str = "Nearby";
/// Searching.
pub const NEARBY_SEARCHING: &str = "Looking for stations on this network…";
/// Nearby not available (browser).
pub const NEARBY_UNAVAILABLE: &str =
    "A browser cannot search the network. Use the link or code from the station.";
/// Pairing open pill.
pub const PAIRING_OPEN: &str = "pairing open";
/// Already paired.
pub const ALREADY_PAIRED: &str = "already paired";
/// Not pairing.
pub const NOT_PAIRING: &str = "pairing closed";
/// Invitation field.
pub const INVITE_FIELD: &str = "Invitation link";
/// Invitation hint.
pub const INVITE_HINT: &str = "Paste the nestwatch:// link or the web address shown on the station.";
/// Invitation submit.
pub const INVITE_SUBMIT: &str = "Pair with link";
/// Code field.
pub const CODE_FIELD: &str = "Pairing code";
/// Code field when a station is chosen.
pub fn code_field_for(name: &str) -> String {
    format!("Pairing code shown on {name}")
}
/// Code submit.
pub const CODE_SUBMIT: &str = "Pair with code";
/// Station ID field (code pairing without discovery).
pub const STATION_ID_FIELD: &str = "Station ID (shown under the code)";
/// Manual address field.
pub const ADDRESS_FIELD: &str = "Station address (optional, for VPN)";
/// Manual address hint.
pub const ADDRESS_HINT: &str = "For example nursery.tailnet.ts.net:47100 or 100.101.102.103:47100.";
/// Pairing in progress.
pub const PAIRING: &str = "Pairing…";
/// Verification prompt.
pub const VERIFY_PROMPT: &str = "Check that the station shows the same number";
/// Verification: same.
pub const VERIFY_SAME: &str = "Same, continue";
/// Verification: different.
pub const VERIFY_DIFFERENT: &str = "Different";
/// After "different".
pub const VERIFY_REMOVED: &str =
    "The station was removed again. Pair once more, close to the station.";
/// Or divider.
pub const OR: &str = "or";
/// Pairing errors.
pub const ERR_INVALID_INVITE: &str = "That link is not a Nestwatch invitation.";
/// Invalid code.
pub const ERR_INVALID_CODE: &str = "That code is not valid. It has 10 letters and digits.";
/// Invalid ID.
pub const ERR_INVALID_ID: &str = "That station ID is not valid.";
/// Invalid address.
pub const ERR_INVALID_ADDRESS: &str = "That address is not valid. Use host:port.";
/// Unreachable.
pub const ERR_UNREACHABLE: &str =
    "The station could not be reached. Is it on and on the same network?";
/// Refused.
pub const ERR_REFUSED: &str = "The station is not pairing right now. Open pairing on the station.";
/// Rejected.
pub const ERR_REJECTED: &str = "Pairing failed. The code may be wrong or the invitation expired.";
/// Too many stations.
pub const ERR_TOO_MANY: &str = "You already follow 8 stations.";
/// Not connected.
pub const ERR_NOT_CONNECTED: &str = "The station is not connected.";
/// Generic failure.
pub const ERR_GENERIC: &str = "Something went wrong.";

// ----- listener home -----------------------------------------------------------

/// No stations yet.
pub const NO_STATIONS: &str =
    "No stations yet. Add the station in your child's room to start listening.";
/// Keeps listening (desktop).
pub const KEEPS_LISTENING: &str = "Nestwatch keeps listening when the window is closed.";
/// Keeps listening only while open (browser).
pub const BROWSER_KEEP_OPEN: &str =
    "Keep this tab open and the device awake: browsers stop listening in the background.";
/// Tap to start audio (browser rule).
pub const TAP_TO_LISTEN: &str = "Tap to start listening";
/// Explanation of the audio prompt.
pub const TAP_TO_LISTEN_HINT: &str = "Browsers only play sound after a tap.";
/// Enable notifications.
pub const ENABLE_NOTIFICATIONS: &str = "Enable notifications";
/// Status: quiet.
pub const STATUS_QUIET: &str = "Quiet";
/// Status: sound.
pub const STATUS_SOUND: &str = "Sound";
/// Status: live in continuous mode.
pub const STATUS_LIVE: &str = "Listening";
/// Status: connecting.
pub const STATUS_CONNECTING: &str = "Connecting";
/// Status: reconnecting.
pub const STATUS_RECONNECTING: &str = "Reconnecting";
/// Status: not followed.
pub const STATUS_PAUSED: &str = "Paused";
/// Status: needs pairing.
pub const STATUS_NEEDS_PAIRING: &str = "Needs pairing";
/// Status: someone talks.
pub fn status_talking(name: Option<&str>) -> String {
    match name {
        Some(n) => format!("{n} talking"),
        None => "Someone talking".into(),
    }
}
/// Status: we talk.
pub const STATUS_YOU_TALK: &str = "You are talking";
/// Path: LAN.
pub const PATH_LAN: &str = "home network";
/// Path: VPN.
pub const PATH_VPN: &str = "via VPN";
/// Trying addresses.
pub fn trying_addresses(n: usize) -> String {
    match n {
        0 => "trying…".into(),
        1 => "trying 1 address…".into(),
        n => format!("trying {n} addresses…"),
    }
}
/// Attempt caption.
pub fn attempt(n: u32) -> String {
    format!("attempt {n}")
}
/// Unreachable caption.
pub fn unreachable_for(duration: &str) -> String {
    format!("Unreachable for {duration}. You'll be alerted when it's back.")
}
/// Needs pairing caption.
pub const NEEDS_PAIRING_HINT: &str = "This station removed this device. Pair again to listen.";
/// Paused caption.
pub const PAUSED_HINT: &str = "Not listening to this station.";
/// Resume.
pub const RESUME: &str = "Resume";
/// Level caption "now".
pub fn level_now(db: &str) -> String {
    format!("{db} now")
}
/// Level caption "room".
pub fn level_room(db: &str) -> String {
    format!("{db} room")
}
/// Threshold caption.
pub fn opens_above(db: &str) -> String {
    format!("opens above {db}")
}
/// Continuous caption.
pub const ALWAYS_ON_CAPTION: &str = "always on";
/// No level yet.
pub const NO_LEVEL: &str = "no level yet";
/// Battery.
pub fn battery(percent: u8) -> String {
    format!("Battery {percent}%")
}
/// Charging.
pub const CHARGING: &str = "charging";
/// Battery and charging.
pub fn battery_charging(percent: u8) -> String {
    format!("Battery {percent}%, charging")
}
/// Mute.
pub const MUTE: &str = "Mute";
/// Solo.
pub const SOLO: &str = "Solo";
/// Camera.
pub const CAMERA: &str = "Camera";
/// Hold to talk.
pub const HOLD_TO_TALK: &str = "Hold to talk";
/// Talking.
pub const TALKING: &str = "Talking… release to stop";
/// Waiting for talk grant.
pub const TALK_WAIT: &str = "Connecting the speaker…";
/// Busy.
pub fn talk_busy(name: Option<&str>) -> String {
    match name {
        Some(n) => format!("{n} is talking"),
        None => "Someone else is talking".into(),
    }
}
/// Talk disabled.
pub const TALK_DISABLED: &str = "Talk-back is off on the station";
/// Talk needs a secure context in the browser.
pub const TALK_NEEDS_HTTPS: &str = "Talk-back needs the station's HTTPS address";
/// Settings of a station (aria).
pub fn station_settings_of(name: &str) -> String {
    format!("Settings for {name}")
}
/// Tile region label.
pub fn tile_label(name: &str, status: &str) -> String {
    format!("{name}: {status}")
}
/// Meter label.
pub const METER_LABEL: &str = "Room level";
/// Alert halo text for screen readers.
pub const SOUND_ALERT_SR: &str = "Sound detected";

// ----- per-station settings -----------------------------------------------------

/// Settings caption.
pub const LISTENING_SETTINGS: &str = "listening settings";
/// Listening mode group.
pub const LISTEN_MODE: &str = "Listening mode";
/// Always on.
pub const MODE_ALWAYS: &str = "Always on";
/// When there's sound.
pub const MODE_SOUND: &str = "When there's sound";
/// Calibration explanation.
pub const CALIBRATE_HINT: &str = "The line is the room's level over the last minute. Drag the threshold just above the quiet part.";
/// Calibration slider label.
pub const THRESHOLD: &str = "Sound threshold";
/// Calibration in continuous mode.
pub const CALIBRATE_CONTINUOUS: &str = "The line is the room's level over the last minute. The threshold only matters for alerts in this mode.";
/// Hold time label.
pub const HOLD_TIME: &str = "Keep listening after sound";
/// Seconds.
pub fn seconds(n: u32) -> String {
    if n == 1 {
        "1 second".into()
    } else {
        format!("{n} seconds")
    }
}
/// Volume.
pub const VOLUME: &str = "Volume";
/// Percent.
pub fn percent(n: u32) -> String {
    format!("{n}%")
}
/// Alert sound toggle.
pub const ALERT_SOUND: &str = "Alert sound";
/// Alert unreachable toggle.
pub const ALERT_UNREACHABLE: &str = "Alert if unreachable";
/// Alert battery toggle.
pub const ALERT_BATTERY: &str = "Alert on low battery";
/// Add an address.
pub const ADD_ADDRESS: &str = "Add an address";
/// Address added.
pub const ADDRESS_ADDED: &str = "Address added.";
/// Pause.
pub const PAUSE: &str = "Stop listening";
/// Forget.
pub const FORGET: &str = "Forget this station";
/// Confirm forget.
pub const FORGET_CONFIRM: &str = "Forget it? You will need to pair again.";
/// Confirm forget button.
pub const FORGET_YES: &str = "Forget";
/// Network route caption.
pub fn connected_via(route: &str) -> String {
    format!("Connected via {route}")
}

// ----- camera ---------------------------------------------------------------

/// Camera view heading.
pub fn camera_of(name: &str) -> String {
    format!("Camera · {name}")
}
/// Camera live badge.
pub const CAMERA_LIVE: &str = "Camera live";
/// Waiting for frames.
pub const CAMERA_STARTING: &str = "Starting the camera…";
/// Station camera off by setting.
pub const CAMERA_DISABLED: &str = "The camera is turned off on the station.";
/// No camera.
pub const CAMERA_UNAVAILABLE: &str = "The station has no camera available.";
/// Permission denied.
pub const CAMERA_DENIED: &str = "The station's camera permission is denied.";
/// Camera stopped.
pub const CAMERA_STOPPED: &str = "The camera stopped.";
/// Night image.
pub const NIGHT_IMAGE: &str = "Night image";
/// Switch camera.
pub const SWITCH_CAMERA: &str = "Switch camera";
/// Camera image alt.
pub fn camera_alt(name: &str) -> String {
    format!("Live camera image of {name}")
}
/// Frame info.
pub fn frame_info(w: u32, h: u32, fps: u32) -> String {
    format!("{w}×{h} · {fps} fps")
}

// ----- alerts ----------------------------------------------------------------

/// Sound alert.
pub fn alert_sound(name: &str) -> String {
    format!("Sound in {name}")
}
/// Unreachable alert.
pub fn alert_unreachable(name: &str) -> String {
    format!("{name} is unreachable")
}
/// Reachable alert.
pub fn alert_reachable(name: &str) -> String {
    format!("{name} is back")
}
/// Battery alert.
pub fn alert_battery(name: &str, percent: u8) -> String {
    format!("{name}: battery low ({percent}%)")
}
/// Station: a listener paired.
pub fn listener_paired(name: &str) -> String {
    format!("{name} paired")
}
/// Alert notification body.
pub const ALERT_BODY: &str = "Open Nestwatch to listen.";

// ----- time ------------------------------------------------------------------

/// Just now.
pub const AGO_NOW: &str = "just now";
/// Minutes ago.
pub fn ago_minutes(n: u64) -> String {
    format!("{n} min ago")
}
/// Hours ago.
pub fn ago_hours(n: u64) -> String {
    format!("{n} h ago")
}
/// Days ago.
pub fn ago_days(n: u64) -> String {
    if n == 1 {
        "yesterday".into()
    } else {
        format!("{n} days ago")
    }
}
/// Paired / last seen caption.
pub fn paired_seen(paired: &str, seen: &str) -> String {
    format!("paired {paired} · last seen {seen}")
}
/// "2 listening".
pub fn listening_count(n: usize) -> String {
    match n {
        0 => "Nobody listening".into(),
        n => format!("{n} listening"),
    }
}
/// Pairing window caption.
pub fn pairing_open_for(remaining: &str, left: u32) -> String {
    let devices = if left == 1 {
        "1 more device".to_string()
    } else {
        format!("up to {left} devices")
    };
    format!("Open for {remaining} · {devices}")
}
/// Station talk indicator.
pub fn talking_through(name: &str) -> String {
    format!("{name} is talking")
}
