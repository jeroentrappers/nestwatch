# Product specification

## 1. Purpose

Lull turns phones, tablets, computers and browsers into a baby monitor. A
**station** sits near the child and captures sound (and, on request, video).
One or more **listeners** follow one or more stations. Everything runs
directly between the user's own devices, on the home network or across a
private VPN the user already operates. There is no account, cloud service,
relay or telemetry.

Lull is not a medical device and does not replace supervision. Every app MUST
say so on first launch and in its about screen.

## 2. Roles and terms

| Term | Meaning |
|------|---------|
| Station | Device in the child's room. Captures audio, optionally video; plays talk-back. |
| Listener | Device a caregiver uses to follow one or more stations. |
| Household | The set of stations and listeners that trust each other. There is no central household object; trust is pairwise. |
| Pairing | One-time introduction that makes a listener trusted by a station. |
| Session | One authenticated connection between a listener and a station. |
| Sound activation | Sending audio to a listener only while the room is louder than that listener's threshold. |
| Talk-back | Listener speaks through the station's speaker. |

Any native app can act as station or listener. The browser app is a listener
only (browsers cannot accept connections or advertise services).

## 3. Functional requirements

### 3.1 Station

- **FR-ST-1** A station MUST capture mono audio continuously while active and
  show a live level meter.
- **FR-ST-2** A station MUST serve at least 8 simultaneously connected
  listeners.
- **FR-ST-3** A station MUST show, at all times while active: how many
  listeners are connected, whether any camera feed is being sent, and
  whether a listener is talking.
- **FR-ST-4** The station's display MUST be dimmable to near-black while
  active (night mode) without stopping monitoring.
- **FR-ST-5** A station MUST keep monitoring with the screen off or the app
  in the background where the platform allows it (see platforms.md), and
  MUST clearly warn when the platform will stop it.
- **FR-ST-6** A station MUST report its battery level and charging state to
  listeners where the platform exposes them.
- **FR-ST-7** A station MUST let the user rename it, list trusted listeners
  (name, last seen) and revoke any of them.
- **FR-ST-8** A station MUST allow turning camera access and talk-back off
  entirely. When off, listeners see the feature as unavailable.

### 3.2 Listener

- **FR-LS-1** A listener MUST be able to follow up to 8 stations at once,
  each with independent connection, volume, mute and alerts.
- **FR-LS-2** Audio of all followed, unmuted stations MUST be mixed into one
  output. A "solo" action MUST let the user hear one station only.
- **FR-LS-3** Each station tile MUST show: name, connection state, live level
  or last reported level, sound-activation state, battery, and the network
  path in use (local network or VPN).
- **FR-LS-4** A listener MUST keep receiving and alerting with the screen off
  or the app in the background where the platform allows it.
- **FR-LS-5** A listener MUST reconnect automatically after network changes,
  trying all known addresses of the station, and MUST show reconnect progress.

### 3.3 Pairing

- **FR-PAIR-1** A station MUST offer an **invitation** as a QR code and as a
  copyable link. Scanning or opening it pairs a listener without typing.
- **FR-PAIR-2** A station MUST offer a **pairing code** of 10 characters for
  devices without a camera. Entering the code pairs a listener found on the
  local network.
- **FR-PAIR-3** Invitations and codes MUST only work while the station is in
  pairing mode, which ends after 10 minutes, when the user closes it, or
  after 8 successful pairings, whichever comes first.
- **FR-PAIR-4** After pairing, the listener MUST reconnect without any code.
  A station's pairing mode MUST NOT be needed for reconnects.
- **FR-PAIR-5** Pairing MUST show both devices a short verification phrase
  derived from the session (see protocol.md §5.5) so users can spot a
  man-in-the-middle when pairing by code.
- **FR-PAIR-6** Revoking a listener on the station MUST end its sessions
  immediately and reject it afterwards. Forgetting a station on a listener
  MUST tell the station (if reachable) so it removes the listener.

### 3.4 Audio and sound activation

- **FR-AU-1** Listeners MUST hear station audio with at most 400 ms mouth-to-ear
  latency on a healthy local network (target 150 ms).
- **FR-AU-2** Each listener MUST be able to set, per station, either
  **continuous** audio or **sound activation** with a threshold and a
  hold time. The threshold UI MUST show the station's current room level so
  the user can place it above the background noise.
- **FR-AU-3** With sound activation, the station MUST NOT send audio to that
  listener while the room stays below the threshold, except for the hold
  time after the last loud moment and a pre-roll of at least 300 ms before
  the trigger, so the onset of a cry is heard.
- **FR-AU-4** The defaults MUST be sound activation at a sensitive threshold
  (`-50 dBFS`), a 4-second hold time and 500 ms pre-roll.
- **FR-AU-5** A station MUST offer microphone sensitivity (gain) settings and
  SHOULD offer automatic gain control, on by default.
- **FR-AU-6** A listener MUST offer per-station volume and mute. Muting MUST
  NOT stop alerts.

### 3.5 Alerts

- **FR-AL-1** A listener MUST alert (sound, vibration where available, and a
  system notification) when a station that was quiet starts sending sound.
  Repeated alerts for the same station MUST be rate-limited (default: not
  more than once per 30 seconds).
- **FR-AL-2** A listener MUST alert when a station has been unreachable for
  longer than 20 seconds, and when it becomes reachable again.
- **FR-AL-3** A listener MUST alert when a station reports battery below 15 %
  while not charging.
- **FR-AL-4** Alerts MUST work while the listener is muted and, where the
  platform allows, while it is in the background.

### 3.6 Talk-back

- **FR-TB-1** A listener MUST offer push-to-talk per station. Only one
  listener may talk to a station at a time; others see who is talking.
- **FR-TB-2** While a listener talks, its own playback of that station MUST
  be muted to avoid feedback.
- **FR-TB-3** A station MUST play talk-back at a user-set volume and MUST
  indicate it visibly.

### 3.7 Camera

- **FR-CAM-1** A listener MUST be able to open a live camera view per station
  on demand. The station's camera MUST be off whenever no listener has an
  open view.
- **FR-CAM-2** The station MUST show a persistent, unmistakable indicator
  while its camera is sending.
- **FR-CAM-3** Video MUST adapt to the link: lower frame rate and quality
  before audio is affected. Audio always has priority over video.
- **FR-CAM-4** A listener MAY switch the station between front and back
  cameras and toggle a night-friendly (grayscale, boosted) image.

### 3.8 Networks

- **FR-NET-1** Stations MUST be discoverable on the local network via mDNS.
- **FR-NET-2** Lull MUST work across a user-operated VPN (for example
  Tailscale or WireGuard) without mDNS: invitations carry all station
  addresses including VPN addresses and VPN DNS names, and listeners MUST
  accept manually entered host names and addresses.
- **FR-NET-3** Listeners MUST remember every address that worked for a
  station and try them in order of last success.
- **FR-NET-4** No component may contact any server other than the user's own
  devices. Updates are delivered by the platform's normal channels.

### 3.9 Browser listener

- **FR-WEB-1** A native station MUST serve the browser listener app and a
  WebSocket endpoint on the local network (see protocol.md §3.3).
- **FR-WEB-2** The browser listener MUST provide listener features except
  background operation; talk-back requires a secure context (HTTPS or
  `localhost`), so stations SHOULD serve HTTPS with a self-signed
  certificate whose fingerprint the station displays.
- **FR-WEB-3** The browser listener MUST store its own device key so that it
  stays paired across reloads.

## 4. Non-functional requirements

- **NFR-LAT-1** Audio latency: see FR-AU-1.
- **NFR-BW-1** Continuous audio MUST fit in 40 kbit/s per listener with the
  preferred codec; the baseline codec MUST fit in 80 kbit/s.
- **NFR-BAT-1** A phone station on mains power MUST run for 12 hours without
  user interaction. A listener with sound activation and screen off SHOULD
  use less than 5 % battery per hour on a modern phone.
- **NFR-REL-1** A listener MUST detect a dead session within 15 seconds.
- **NFR-PRIV-1** No audio, video, level data or identifiers may leave the
  household's devices. No analytics, no crash upload.
- **NFR-A11Y-1** Every screen MUST work with the platform screen reader and
  with large text; status MUST never rely on colour alone.
- **NFR-I18N-1** User-facing text MUST be externalised for translation;
  English is the reference language.

## 5. Out of scope (version 1)

Cloud relays or accounts, recording or history of audio/video, breathing or
movement analysis, smart-home integrations, lullaby playback, multi-household
sharing beyond pairwise pairing.
