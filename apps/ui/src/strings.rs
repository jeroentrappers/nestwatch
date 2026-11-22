// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every user-facing string of the Dioxus apps (product.md NFR-I18N-1/2),
//! declared once with its English text. Components never contain literal
//! text.
//!
//! Each entry becomes a function of the same name that returns the text in
//! the current language ([`crate::i18n`]): `fn close() = "Close";` gives
//! `close() -> &'static str`; `fn alert_sound(name: &str) = "Sound in {name}";`
//! gives `alert_sound(name) -> String` (named placeholders, the argument
//! names); `fn wake_ups(n: u64) = { one: "…", other: "…" };` is a plural
//! chosen by the first argument with the language's plural rules.
//!
//! To add a string: add an entry here (English, with a doc comment that
//! tells translators where it shows), then add the key to every file in
//! `apps/ui/i18n/` (the unit test `every_language_is_complete_and_consistent`
//! lists what is missing).
//!
//! Voice (design README §2): short, kind, specific.

use crate::i18n::{self, Reference};

macro_rules! string_fn {
    ([$($doc:literal),*] $name:ident () $val:literal) => {
        $(#[doc = $doc])*
        pub fn $name() -> &'static str {
            i18n::text(stringify!($name), $val)
        }
    };
    ([$($doc:literal),*] $name:ident ($count:ident : $cty:ty $(, $arg:ident : $ty:ty)*)
        { $($cat:ident : $form:literal),+ $(,)? }) => {
        $(#[doc = $doc])*
        pub fn $name($count: $cty $(, $arg: $ty)*) -> String {
            i18n::plural(
                stringify!($name),
                $count,
                &[$((stringify!($cat), $form)),+],
                &[
                    (stringify!($count), &$count as &dyn std::fmt::Display)
                    $(, (stringify!($arg), &$arg as &dyn std::fmt::Display))*
                ],
            )
        }
    };
    ([$($doc:literal),*] $name:ident ($($arg:ident : $ty:ty),+) $val:literal) => {
        $(#[doc = $doc])*
        pub fn $name($($arg: $ty),+) -> String {
            i18n::format(stringify!($name), $val, &[$((stringify!($arg), &$arg as &dyn std::fmt::Display)),+])
        }
    };
}

macro_rules! string_ref {
    ($name:ident, $doc:expr, $val:literal) => {
        Reference {
            key: stringify!($name),
            doc: $doc,
            plural: false,
            forms: vec![("", $val)],
        }
    };
    ($name:ident, $doc:expr, { $($cat:ident : $form:literal),+ $(,)? }) => {
        Reference {
            key: stringify!($name),
            doc: $doc,
            plural: true,
            forms: vec![$((stringify!($cat), $form)),+],
        }
    };
}

macro_rules! strings {
    ($( $(#[doc = $doc:literal])* fn $name:ident ( $($args:tt)* ) = $val:tt ; )*) => {
        $( string_fn!([$($doc),*] $name ( $($args)* ) $val); )*

        /// Every string with its English text, in declaration order
        /// (translation checks and templates).
        pub fn reference() -> Vec<Reference> {
            vec![ $( string_ref!($name, concat!($($doc),*), $val) ),* ]
        }
    };
}

/// Status: someone talks.
pub fn status_talking(name: Option<&str>) -> String {
    match name {
        Some(n) => status_talking_named(n),
        None => status_talking_someone().to_owned(),
    }
}

/// Trying addresses.
pub fn trying_addresses(n: usize) -> String {
    match n {
        0 => trying_no_address().to_owned(),
        n => trying_addresses_n(n as u64),
    }
}

/// Busy.
pub fn talk_busy(name: Option<&str>) -> String {
    match name {
        Some(n) => talk_busy_named(n),
        None => talk_busy_someone().to_owned(),
    }
}

/// "2 listening".
pub fn listening_count(n: usize) -> String {
    match n {
        0 => listening_nobody().to_owned(),
        n => listening_n(n as u64),
    }
}

/// Pairing window caption.
pub fn pairing_open_for(remaining: &str, left: u32) -> String {
    pairing_open_window(remaining, &pairing_devices_left(u64::from(left)))
}

/// Short weekday name, 0 = Sunday.
pub fn weekday_short(d: u32) -> &'static str {
    [
        weekday_sun,
        weekday_mon,
        weekday_tue,
        weekday_wed,
        weekday_thu,
        weekday_fri,
        weekday_sat,
    ][(d % 7) as usize]()
}

/// Short month name, 1 = January.
pub fn month_short(m: u32) -> &'static str {
    [
        month_jan, month_feb, month_mar, month_apr, month_may, month_jun, month_jul, month_aug,
        month_sep, month_oct, month_nov, month_dec,
    ][(m.clamp(1, 12) - 1) as usize]()
}

strings! {
    /// Product name.
    fn app() = "Nestwatch";
    /// Promise on the welcome screen.
    fn promise() = "A private baby monitor for the devices you already own. Everything stays on your network.";
    /// Medical disclaimer (product.md §1), first launch and About.
    fn disclaimer() = "Nestwatch is not a medical device and does not replace supervision. Always check on your child in person.";
    /// Accept the disclaimer.
    fn disclaimer_accept() = "I understand";
    /// Role choice: station.
    fn role_station() = "This device watches";
    /// Explanation of the station role.
    fn role_station_hint() = "Put it in the child's room. It listens and, when asked, shows the camera.";
    /// Role choice: listener.
    fn role_listener() = "This device listens";
    /// Explanation of the listener role.
    fn role_listener_hint() = "Follow one or more rooms from here.";
    /// Role switch group label.
    fn role_group() = "Role";
    /// Role switch: listener.
    fn role_tab_listening() = "Listening";
    /// Role switch: station.
    fn role_tab_station() = "Station";
    /// Browser cannot be a station.
    fn role_station_unavailable() = "A browser can only listen. Use the Nestwatch app on a phone or computer as the station.";
    /// The browser's entry to the station role (product.md FR-WEBST-1).
    fn station_use_this_device() = "Use this device as a station";
    /// Explanation of the browser station role.
    fn role_station_hint_web() = "Put this device in the child's room. Listeners reach it through the relay server; keep this tab open.";

    // ----- browser station (product.md §3.10) -----------------------------------

    /// Always shown while a browser station runs (FR-WEBST-9).
    fn station_web_keep_visible() = "Keep this tab open and visible. Phones pause hidden tabs, and closing or reloading the tab stops the station.";
    /// Wake lock held (FR-WEBST-8).
    fn wake_lock_on() = "The screen stays on while the station runs.";
    /// No wake lock (FR-WEBST-8).
    fn wake_lock_off() = "This browser cannot keep the screen on: turn off auto-lock, or keep the screen on yourself.";
    /// The station's sound waits for a tap (FR-WEBST-6).
    fn station_tap_to_start() = "Tap to start the microphone and speaker";
    /// Explanation of the station's tap prompt.
    fn station_tap_hint() = "Browsers only record and play sound after a tap.";
    /// No microphone (FR-WEBST-4).
    fn mic_unavailable() = "No microphone access: the station sends silence. Allow the microphone for this site, then stop and start the station.";
    /// Station footnote in the browser (FR-WEBST-11).
    fn station_keeps_running_web() = "The station keeps running while you switch to Listening. Closing or reloading this tab stops it.";
    /// Relay section of a relay-only station (rendezvous.md FR-RV-7).
    fn relay_only_note() = "A browser station is reachable through the relay server only, so this setting stays on.";
    /// The relay server is the page's origin (FR-WEBST-2).
    fn relay_server_fixed() = "This station uses the server this page comes from.";
    /// Pairing codes need the local network (OPEN-QUESTIONS WEBST-1).
    fn pair_code_relay_only() = "Pairing codes need the local network: scan the QR code or open a link instead.";
    /// Where a browser station keeps its keys (security.md §7.1).
    fn browser_station_storage() = "This station's keys are kept in this browser, for this site only. Clearing the site's data or closing a private window removes the station; its listeners then need to pair again.";
    /// Default name of a browser station.
    fn browser_station_name(browser: &str) = "{browser} station";
    /// A configurable server the page may not be allowed to reach (WEBST-8).
    fn relay_open_app_hint(server: &str) = "If the relay stays unreachable, this page may not be allowed to connect to it: open {server}/app/ and start the station there.";

    // ----- common ---------------------------------------------------------------

    /// Generic close.
    fn close() = "Close";
    /// Back.
    fn back() = "Back";
    /// Done.
    fn done() = "Done";
    /// Cancel.
    fn cancel() = "Cancel";
    /// Save.
    fn save() = "Save";
    /// Settings button label.
    fn settings() = "Settings";
    /// About.
    fn about() = "About Nestwatch";
    /// Copy.
    fn copy() = "Copy link";
    /// Copied.
    fn copied() = "Copied";
    /// Theme group label.
    fn theme() = "Theme";
    /// Theme: follow the system.
    fn theme_system() = "System";
    /// Theme: night.
    fn theme_night() = "Night";
    /// Theme: day.
    fn theme_day() = "Day";
    /// Dismiss a notice.
    fn dismiss() = "Dismiss";
    /// Alerts region label.
    fn notices() = "Alerts";
    /// Unknown name placeholder.
    fn unnamed() = "Unnamed device";
    /// About text.
    fn about_body() = "Nestwatch connects your own devices directly, on your home network or your own VPN. There is no account, no cloud and no tracking. Audio and video are end-to-end encrypted.";
    /// About: version line prefix.
    fn version() = "Version";
    /// About: device ID label.
    fn device_id() = "Device ID";
    /// About: licence.
    fn license() = "Free software under the GNU AGPL 3.0 or later.";

    // ----- station --------------------------------------------------------------

    /// Station is starting.
    fn station_starting() = "Starting the station…";
    /// Station could not start.
    fn station_failed() = "The station could not start";
    /// Retry.
    fn retry() = "Try again";
    /// Station not running.
    fn station_stopped() = "The station is not running.";
    /// Start the station.
    fn station_start() = "Start the station";
    /// Stop the station.
    fn station_stop() = "Stop the station";
    /// Pair a listener.
    fn pair_listener() = "Pair a listener";
    /// Dim screen.
    fn dim_screen() = "Dim screen";
    /// Tap to brighten.
    fn tap_to_brighten() = "Tap anywhere to brighten";
    /// Station screen is dimmed (screen reader).
    fn dimmed_label() = "Screen dimmed. Monitoring continues. Activate to brighten.";
    /// No listeners yet.
    fn no_listeners() = "Nobody is listening yet.";
    /// Connected listeners heading (screen reader).
    fn listeners_heading() = "Listening now";
    /// Mode: sound activated.
    fn mode_sound_activated() = "sound-activated";
    /// Mode: always on.
    fn mode_always_on() = "always on";
    /// Listener uses the browser.
    fn via_browser() = "browser";
    /// Listener row: connected through the relay server.
    fn via_relay() = "relayed";
    /// Hearing now.
    fn hearing() = "hearing the room";
    /// Talking now.
    fn talking_now() = "talking";
    /// Watching now.
    fn watching_now() = "camera";
    /// Ring caption: quiet.
    fn ring_quiet() = "quiet";
    /// Ring caption: sound.
    fn ring_sound() = "sound";
    /// Ring unit.
    fn db() = "dB";
    /// Ring label for screen readers.
    fn ring_label() = "Room level";
    /// Camera on indicator, nobody named.
    fn camera_on() = "Camera on";
    /// Camera off indicator.
    fn camera_off() = "Camera off";
    /// Talk-back off.
    fn talk_off() = "Talk-back off";
    /// Station background warning (FR-ST-5).
    fn station_keeps_running() = "Keep this app open. Closing the window keeps the station running in the tray; quitting stops monitoring.";
    /// Station settings heading.
    fn station_settings() = "Station settings";
    /// Station name field.
    fn station_name() = "Station name";
    /// Camera access switch.
    fn camera_allowed() = "Allow the camera";
    /// Talk-back switch.
    fn talk_allowed() = "Allow talk-back";
    /// Gain field.
    fn mic_sensitivity() = "Microphone sensitivity";
    /// Gain: automatic.
    fn gain_auto() = "Auto";
    /// Talk-back volume.
    fn talkback_volume() = "Talk-back volume";
    /// Trusted listeners heading.
    fn trusted_listeners() = "Paired listeners";
    /// No trusted listeners.
    fn no_trusted() = "No listener is paired yet.";
    /// Revoke.
    fn revoke() = "Remove";
    /// HTTPS fingerprint heading.
    fn https_fingerprint() = "HTTPS certificate fingerprint";
    /// Fingerprint explanation.
    fn https_fingerprint_hint() = "Browsers warn about the station's own certificate. Accept it only if the fingerprint matches.";
    /// Browser address heading.
    fn browser_address() = "Browser address";
    /// Camera unavailable on this station.
    fn camera_unavailable_here() = "No camera found";

    // ----- relay (station, rendezvous.md FR-RV-1, §7) ----------------------------

    /// Relay setting.
    fn relay_title() = "Reachable through a relay server";
    /// What the relay is for.
    fn relay_intro() = "Lets your listeners reach this station when a direct connection is not possible, for example on mobile data, guest Wi-Fi or another network. On your home network nothing changes.";
    /// What the server operator can see (rendezvous.md §7).
    fn relay_can_see() = "The server can see that this station is online, its public IP address, which IP addresses connect to it and when, and how much data flows.";
    /// What the server cannot see (rendezvous.md §7).
    fn relay_cannot_see() = "It cannot see or hear anything else: audio, video, messages, device names, pairing codes and keys are encrypted end to end between your devices, and invitations never pass through it.";
    /// Traffic analysis note (rendezvous.md §7).
    fn relay_traffic_note() = "With sound activation, the amount of data can show when there is sound in the room.";
    /// Heading of the "can see" paragraph.
    fn relay_can_see_heading() = "What the server can see";
    /// Heading of the "cannot see" paragraph.
    fn relay_cannot_see_heading() = "What it cannot see";
    /// Server address field.
    fn relay_server() = "Relay server";
    /// Server address hint (FR-RV-6).
    fn relay_server_hint() = "Keep the Nestwatch server or enter the address of your own (https://…).";
    /// Invalid server address.
    fn relay_server_invalid() = "Enter a server address like https://relay.example.org";
    /// The server set by default.
    fn relay_server_named(server: &str) = "Server: {server}";
    /// Status pill: off.
    fn relay_off() = "Relay off";
    /// Status pill: connecting.
    fn relay_connecting() = "Connecting to relay…";
    /// Status pill: registered.
    fn relay_online() = "Reachable via relay";
    /// Status pill: failed.
    fn relay_error() = "Relay unreachable";
    /// Status pill: refused for good.
    fn relay_refused() = "Relay refused this station";
    /// Hint for a permanent refusal.
    fn relay_refused_hint() = "Reset the relay identity or choose another server.";
    /// Reset relay identity.
    fn relay_reset() = "Reset relay identity";
    /// What resetting means.
    fn relay_reset_hint() = "Creates a new relay address for this station. Listeners that use the relay need a new invitation.";
    /// Confirm the reset.
    fn relay_reset_confirm() = "Reset now";
    /// Setup question (asked once).
    fn relay_setup_title() = "Reach this station from anywhere?";
    /// Setup: opt in.
    fn relay_setup_yes() = "Use the relay server";
    /// Setup: keep it off.
    fn relay_setup_no() = "Not now";
    /// Setup footnote.
    fn relay_setup_later() = "You can change this at any time in the station settings.";
    /// Pairing sheet: the relay link.
    fn pair_relay_link() = "Link via relay";
    /// Camera permission denied on this station.
    fn camera_denied_here() = "Camera permission denied";

    // ----- pairing sheet (station) ------------------------------------------------

    /// Pairing sheet intro.
    fn pair_intro() = "Scan with the Nestwatch app, or open the link on a computer.";
    /// Code intro.
    fn pair_code_intro() = "No camera? Enter this code";
    /// Station ID caption on the sheet (for code pairing in a browser).
    fn pair_station_id() = "Station ID";
    /// QR code label.
    fn qr_label() = "Pairing QR code";
    /// Pairing is opening.
    fn pair_opening() = "Opening pairing…";
    /// Pairing ended.
    fn pair_closed() = "Pairing is closed.";
    /// Paired listeners heading on the sheet.
    fn pair_just_paired() = "Paired now. Check that the listener shows the same number:";
    /// Link label.
    fn pair_link() = "Web link";
    /// The `nestwatch://` invitation link, when the station has no web address.
    fn pair_app_link() = "App link";

    // ----- add a station (listener) ----------------------------------------------

    /// Add a station.
    fn add_station() = "Add a station";
    /// Add station button.
    fn add_station_button() = "+ Add station";
    /// Nearby heading.
    fn nearby() = "Nearby";
    /// Searching.
    fn nearby_searching() = "Looking for stations on this network…";
    /// Nearby not available (browser).
    fn nearby_unavailable() = "A browser cannot search the network. Use the link or code from the station.";
    /// Pairing open pill.
    fn pairing_open() = "pairing open";
    /// Already paired.
    fn already_paired() = "already paired";
    /// Not pairing.
    fn not_pairing() = "pairing closed";
    /// Invitation field.
    fn invite_field() = "Invitation link";
    /// Invitation hint.
    fn invite_hint() = "Paste the nestwatch:// link or the web address shown on the station.";
    /// Invitation submit.
    fn invite_submit() = "Pair with link";
    /// Code field.
    fn code_field() = "Pairing code";
    /// Code field when a station is chosen.
    fn code_field_for(name: &str) = "Pairing code shown on {name}";
    /// Code submit.
    fn code_submit() = "Pair with code";
    /// Station ID field (code pairing without discovery).
    fn station_id_field() = "Station ID (shown under the code)";
    /// Manual address field.
    fn address_field() = "Station address (optional, for VPN)";
    /// Manual address hint.
    fn address_hint() = "For example nursery.tailnet.ts.net:47100 or 100.101.102.103:47100.";
    /// Pairing in progress.
    fn pairing() = "Pairing…";
    /// Verification prompt.
    fn verify_prompt() = "Check that the station shows the same number";
    /// Verification: same.
    fn verify_same() = "Same, continue";
    /// Verification: different.
    fn verify_different() = "Different";
    /// After "different".
    fn verify_removed() = "The station was removed again. Pair once more, close to the station.";
    /// Or divider.
    fn or() = "or";
    /// Pairing errors.
    fn err_invalid_invite() = "That link is not a Nestwatch invitation.";
    /// Invalid code.
    fn err_invalid_code() = "That code is not valid. It has 10 letters and digits.";
    /// Invalid ID.
    fn err_invalid_id() = "That station ID is not valid.";
    /// Invalid address.
    fn err_invalid_address() = "That address is not valid. Use host:port.";
    /// Unreachable.
    fn err_unreachable() = "The station could not be reached. Is it on and on the same network?";
    /// Refused.
    fn err_refused() = "The station is not pairing right now. Open pairing on the station.";
    /// Rejected.
    fn err_rejected() = "Pairing failed. The code may be wrong or the invitation expired.";
    /// Too many stations.
    fn err_too_many() = "You already follow 8 stations.";
    /// Not connected.
    fn err_not_connected() = "The station is not connected.";
    /// Generic failure.
    fn err_generic() = "Something went wrong.";

    // ----- listener home -----------------------------------------------------------

    /// No stations yet.
    fn no_stations() = "No stations yet. Add the station in your child's room to start listening.";
    /// Keeps listening (desktop).
    fn keeps_listening() = "Nestwatch keeps listening when the window is closed.";
    /// Keeps listening only while open (browser).
    fn browser_keep_open() = "Keep this tab open and the device awake: browsers stop listening in the background.";
    /// Tap to start audio (browser rule).
    fn tap_to_listen() = "Tap to start listening";
    /// Explanation of the audio prompt.
    fn tap_to_listen_hint() = "Browsers only play sound after a tap.";
    /// Enable notifications.
    fn enable_notifications() = "Enable notifications";
    /// Status: quiet.
    fn status_quiet() = "Quiet";
    /// Status: sound.
    fn status_sound() = "Sound";
    /// Status: live in continuous mode.
    fn status_live() = "Listening";
    /// Status: connecting.
    fn status_connecting() = "Connecting";
    /// Status: reconnecting.
    fn status_reconnecting() = "Reconnecting";
    /// Status: not followed.
    fn status_paused() = "Paused";
    /// Status: needs pairing.
    fn status_needs_pairing() = "Needs pairing";
    /// Status: someone talks.
    fn status_talking_named(name: &str) = "{name} talking";
    /// Status: someone (unnamed) talks.
    fn status_talking_someone() = "Someone talking";
    /// Status: we talk.
    fn status_you_talk() = "You are talking";
    /// Path: LAN.
    fn path_lan() = "home network";
    /// Path: VPN.
    fn path_vpn() = "via VPN";
    /// Path: hole-punched across the internet (rendezvous.md FR-RV-4).
    fn path_direct() = "direct (internet)";
    /// Path: through the relay server (rendezvous.md FR-RV-4).
    fn path_relayed() = "relayed";
    /// Trying addresses.
    fn trying_no_address() = "trying…";
    /// Trying several addresses.
    fn trying_addresses_n(n: u64) = { one: "trying {n} address…", other: "trying {n} addresses…" };
    /// Attempt caption.
    fn attempt(n: u32) = "attempt {n}";
    /// Unreachable caption.
    fn unreachable_for(duration: &str) = "Unreachable for {duration}. You'll be alerted when it's back.";
    /// Needs pairing caption.
    fn needs_pairing_hint() = "This station removed this device. Pair again to listen.";
    /// Paused caption.
    fn paused_hint() = "Not listening to this station.";
    /// Resume.
    fn resume() = "Resume";
    /// Level caption "now".
    fn level_now(db: &str) = "{db} now";
    /// Level caption "room".
    fn level_room(db: &str) = "{db} room";
    /// Threshold caption.
    fn opens_above(db: &str) = "opens above {db}";
    /// Continuous caption.
    fn always_on_caption() = "always on";
    /// No level yet.
    fn no_level() = "no level yet";
    /// Battery.
    fn battery(percent: u8) = "Battery {percent}%";
    /// Charging.
    fn charging() = "charging";
    /// Battery and charging.
    fn battery_charging(percent: u8) = "Battery {percent}%, charging";
    /// Mute.
    fn mute() = "Mute";
    /// Solo.
    fn solo() = "Solo";
    /// Camera.
    fn camera() = "Camera";
    /// Hold to talk.
    fn hold_to_talk() = "Hold to talk";
    /// Talking.
    fn talking() = "Talking… release to stop";
    /// Waiting for talk grant.
    fn talk_wait() = "Connecting the speaker…";
    /// Busy.
    fn talk_busy_named(name: &str) = "{name} is talking";
    /// Busy, talker unknown.
    fn talk_busy_someone() = "Someone else is talking";
    /// Talk disabled.
    fn talk_disabled() = "Talk-back is off on the station";
    /// Talk needs a secure context in the browser.
    fn talk_needs_https() = "Talk-back needs the station's HTTPS address";
    /// Settings of a station (aria).
    fn station_settings_of(name: &str) = "Settings for {name}";
    /// Tile region label.
    fn tile_label(name: &str, status: &str) = "{name}: {status}";
    /// Meter label.
    fn meter_label() = "Room level";
    /// Alert halo text for screen readers.
    fn sound_alert_sr() = "Sound detected";

    // ----- per-station settings -----------------------------------------------------

    /// Settings caption.
    fn listening_settings() = "listening settings";
    /// Listening mode group.
    fn listen_mode() = "Listening mode";
    /// Always on.
    fn mode_always() = "Always on";
    /// When there's sound.
    fn mode_sound() = "When there's sound";
    /// Calibration explanation.
    fn calibrate_hint() = "The line is the room's level over the last minute. Drag the threshold just above the quiet part.";
    /// Calibration slider label.
    fn threshold() = "Sound threshold";
    /// Calibration in continuous mode.
    fn calibrate_continuous() = "The line is the room's level over the last minute. The threshold only matters for alerts in this mode.";
    /// Hold time label.
    fn hold_time() = "Keep listening after sound";
    /// Seconds.
    fn seconds(n: u64) = { one: "{n} second", other: "{n} seconds" };
    /// Volume.
    fn volume() = "Volume";
    /// Percent.
    fn percent(n: u32) = "{n}%";
    /// Alert sound toggle.
    fn alert_sound_toggle() = "Alert sound";
    /// Alert unreachable toggle.
    fn alert_unreachable_toggle() = "Alert if unreachable";
    /// Alert battery toggle.
    fn alert_battery_toggle() = "Alert on low battery";
    /// Add an address.
    fn add_address() = "Add an address";
    /// Address added.
    fn address_added() = "Address added.";
    /// Pause.
    fn pause() = "Stop listening";
    /// Forget.
    fn forget() = "Forget this station";
    /// Confirm forget.
    fn forget_confirm() = "Forget it? You will need to pair again.";
    /// Confirm forget button.
    fn forget_yes() = "Forget";
    /// Network route caption.
    fn connected_via(route: &str) = "Connected via {route}";

    // ----- camera ---------------------------------------------------------------

    /// Camera view heading.
    fn camera_of(name: &str) = "Camera · {name}";
    /// Camera live badge.
    fn camera_live() = "Camera live";
    /// Waiting for frames.
    fn camera_starting() = "Starting the camera…";
    /// Station camera off by setting.
    fn camera_disabled() = "The camera is turned off on the station.";
    /// No camera.
    fn camera_unavailable() = "The station has no camera available.";
    /// Permission denied.
    fn camera_denied() = "The station's camera permission is denied.";
    /// Camera stopped.
    fn camera_stopped() = "The camera stopped.";
    /// Night image.
    fn night_image() = "Night image";
    /// Switch camera.
    fn switch_camera() = "Switch camera";
    /// Camera image alt.
    fn camera_alt(name: &str) = "Live camera image of {name}";
    /// Frame info.
    fn frame_info(w: u32, h: u32, fps: u32) = "{w}×{h} · {fps} fps";

    // ----- alerts ----------------------------------------------------------------

    /// Sound alert.
    fn alert_sound(name: &str) = "Sound in {name}";
    /// Unreachable alert.
    fn alert_unreachable(name: &str) = "{name} is unreachable";
    /// Reachable alert.
    fn alert_reachable(name: &str) = "{name} is back";
    /// Battery alert.
    fn alert_battery(name: &str, percent: u8) = "{name}: battery low ({percent}%)";
    /// Station: a listener paired.
    fn listener_paired(name: &str) = "{name} paired";
    /// Alert notification body.
    fn alert_body() = "Open Nestwatch to listen.";

    // ----- time ------------------------------------------------------------------

    /// Just now.
    fn ago_now() = "just now";
    /// Minutes ago.
    fn ago_minutes(n: u64) = "{n} min ago";
    /// Hours ago.
    fn ago_hours(n: u64) = "{n} h ago";
    /// Days ago.
    fn ago_days(n: u64) = { one: "yesterday", other: "{n} days ago" };
    /// Paired / last seen caption.
    fn paired_seen(paired: &str, seen: &str) = "paired {paired} · last seen {seen}";
    /// "2 listening".
    fn listening_nobody() = "Nobody listening";
    /// "2 listening".
    fn listening_n(n: u64) = { one: "{n} listening", other: "{n} listening" };
    /// Pairing window caption.
    fn pairing_open_window(remaining: &str, devices: &str) = "Open for {remaining} · {devices}";
    /// How many more devices may pair in this window.
    fn pairing_devices_left(n: u64) = { one: "{n} more device", other: "up to {n} devices" };
    /// Station talk indicator.
    fn talking_through(name: &str) = "{name} is talking";

    // ----- alarm, room status and night log (FR-AL-5..7, §3.11) ------------

    /// Button on the alert banner and notifications: acknowledges the alert
    /// or alarm ("I am going to the child"). Very short.
    fn im_on_it() = "I'm on it";
    /// Button and badge: the sound was not the child (a dog, a TV…); it
    /// pauses sound alerts for 10 minutes. Also a badge in the night log.
    fn not_the_baby() = "Not the baby";
    /// Button that ends the 10-minute pause after "Not the baby".
    fn resume_alerts() = "Resume alerts";
    /// Shown on the tile while alerts are paused; `time` is local time (03:40).
    fn alerts_paused_until(time: &str) = "Alerts paused until {time}";
    /// System notification title when the escalating alarm rings.
    fn alarm_title(name: &str) = "{name}: the sound goes on";
    /// System notification body of the alarm.
    fn alarm_body() = "Nobody answered the alert yet.";
    /// Prominent banner on the station tile while the alarm rings.
    fn alarm_banner(name: &str) = "Alarm · {name}";
    /// Second line of the ringing alarm banner.
    fn alarm_banner_hint() = "The sound goes on and nobody answered yet.";
    /// Room status on the tile: a wake-up has been going on for `duration`.
    fn awake_for(duration: &str) = "Awake for {duration}";
    /// Room status on the tile: no wake-up for `duration`.
    fn quiet_for(duration: &str) = "Quiet for {duration}";
    /// A duration under one minute ("Awake for less than a minute").
    fn less_than_a_minute() = "less than a minute";
    /// A duration in seconds, short ("30 s").
    fn duration_seconds(n: u32) = "{n} s";
    /// A duration in minutes, short ("4 min").
    fn duration_minutes(n: u64) = "{n} min";
    /// A duration in whole hours, short ("2 h").
    fn duration_hours(n: u64) = "{n} h";
    /// A duration in hours and minutes, short ("1 h 20 min").
    fn duration_hours_minutes(h: u64, m: u64) = "{h} h {m} min";
    /// Night log screen title and the button that opens it.
    fn night_log() = "Night log";
    /// Night log heading with the station name.
    fn night_log_of(name: &str) = "Night log · {name}";
    /// Heading of one night (noon to noon) in the night log; `date` is the
    /// evening's date ("Sat 21 Nov").
    fn night_of(date: &str) = "Night of {date}";
    /// Number of wake-ups in one night.
    fn wake_ups(n: u64) = { one: "{n} wake-up", other: "{n} wake-ups" };
    /// Longest time without a wake-up in one night, after the count
    /// ("3 wake-ups · longest quiet 4 h 10 min").
    fn longest_quiet(duration: &str) = "longest quiet {duration}";
    /// One wake-up in the night log: start time and how long it lasted.
    fn wake_up_entry(time: &str, duration: &str) = "{time} · {duration}";
    /// Marks the wake-up that is still going on in the night log.
    fn going_on() = "going on";
    /// Badge: this wake-up made the alarm ring.
    fn badge_alarm() = "Alarm";
    /// Night log without entries.
    fn night_log_empty() = "No wake-ups yet. A wake-up is noted when the room is loud for 10 seconds or more.";
    /// Night log footer.
    fn night_log_footer() = "Kept on this device only, for 30 days. Nestwatch listens for sound; a quiet room does not mean your child is asleep.";
    /// Button that clears the night log of a station.
    fn clear_log() = "Clear log";
    /// Confirmation question before clearing.
    fn clear_log_confirm(name: &str) = "Clear the night log of {name}?";
    /// Confirm button for clearing.
    fn clear() = "Clear";
    /// Alert setting: escalate unanswered sound alerts to an alarm.
    fn escalating_alarm() = "Escalating alarm";
    /// Explanation under the escalating alarm setting.
    fn escalating_alarm_hint() = "If the sound goes on and nobody answers the alert, an alarm rings, louder and louder, also in silent mode.";
    /// Choice: how long after an unanswered sound alert the alarm rings.
    fn alarm_after() = "Alarm after";

    // ----- dates and times ---------------------------------------------------

    /// Time of day, 24 hours; `hh` and `mm` are two digits ("03:40").
    fn time_hm(hh: &str, mm: &str) = "{hh}:{mm}";
    /// Short date with weekday ("Sat 21 Nov"); `day` is the day of the month.
    fn date_short(weekday: &str, day: u32, month: &str) = "{weekday} {day} {month}";
    /// Short weekday name.
    fn weekday_sun() = "Sun";
    /// Short weekday name.
    fn weekday_mon() = "Mon";
    /// Short weekday name.
    fn weekday_tue() = "Tue";
    /// Short weekday name.
    fn weekday_wed() = "Wed";
    /// Short weekday name.
    fn weekday_thu() = "Thu";
    /// Short weekday name.
    fn weekday_fri() = "Fri";
    /// Short weekday name.
    fn weekday_sat() = "Sat";
    /// Short month name.
    fn month_jan() = "Jan";
    /// Short month name.
    fn month_feb() = "Feb";
    /// Short month name.
    fn month_mar() = "Mar";
    /// Short month name.
    fn month_apr() = "Apr";
    /// Short month name.
    fn month_may() = "May";
    /// Short month name.
    fn month_jun() = "Jun";
    /// Short month name.
    fn month_jul() = "Jul";
    /// Short month name.
    fn month_aug() = "Aug";
    /// Short month name.
    fn month_sep() = "Sep";
    /// Short month name.
    fn month_oct() = "Oct";
    /// Short month name.
    fn month_nov() = "Nov";
    /// Short month name.
    fn month_dec() = "Dec";

    // ----- desktop tray --------------------------------------------------------

    /// Tray menu: show the window.
    fn tray_show() = "Show Nestwatch";
    /// Tray menu: quit the app.
    fn tray_quit() = "Quit Nestwatch";
}
