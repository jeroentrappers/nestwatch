// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wire format of the Nestwatch Wire Protocol 1 (NWP/1, `specs/protocol.md`).
//!
//! Sans-I/O and pure: no sockets, async or cryptography. Covers the preface
//! (§4), framing (§3.1, §3.3), records (§6), control messages (§7),
//! endpoints and invitations (§3.5) and discovery TXT data (§3.4).

mod cbor;
pub mod control;
pub mod device_id;
pub mod discovery;
pub mod endpoint;
pub mod frame;
pub mod invite;
pub mod preface;
pub mod record;
pub mod video;

pub use control::{Control, ControlError};
pub use device_id::DeviceId;
pub use discovery::StationTxt;
pub use endpoint::{Endpoint, Host};
pub use frame::{FrameDecoder, FrameError};
pub use invite::{Invitation, InviteError, RelayRef};
pub use preface::{ListenerPreface, Mode, PrefaceError, StationPreface, Status};
pub use record::{AudioHeader, Record, RecordError, VideoHeader};
pub use video::{Reassembler, VideoError, VideoFrame};
