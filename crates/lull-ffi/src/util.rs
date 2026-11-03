// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stateless helpers for the apps.

use crate::error::LullError;

/// A QR code as a module matrix (product.md FR-PAIR-1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QrMatrix {
    /// Side length in modules, without a quiet zone (add ≥ 2 modules of
    /// light margin when drawing).
    pub size: u32,
    /// Row-major modules, `1` dark, `0` light; `size * size` bytes.
    pub modules: Vec<u8>,
}

/// Encodes `text` as a QR code with error correction level M; `None` if
/// it does not fit.
#[uniffi::export]
pub fn qr_code(text: String) -> Option<QrMatrix> {
    let code =
        qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    let size = code.width();
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| u8::from(c == qrcode::Color::Dark))
        .collect();
    Some(QrMatrix {
        size: size as u32,
        modules,
    })
}

/// Formats a device ID for display, `abcd-efgh-ijkl-mnop` (protocol.md
/// §2.2).
#[uniffi::export]
pub fn grouped_id(id: String) -> String {
    lull_core::grouped_id(&id)
}

/// Canonical form of a device ID typed by the user (grouping, case and
/// spaces removed), or an error.
#[uniffi::export]
pub fn parse_device_id(input: String) -> Result<String, LullError> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    lull_proto::DeviceId::parse(&cleaned.to_ascii_lowercase())
        .map(|d| d.as_str().to_owned())
        .map_err(|_| LullError::Invalid {
            reason: "station id".into(),
        })
}

/// What an invitation says before pairing with it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InvitationInfo {
    /// Station display name.
    pub name: String,
    /// Station device ID.
    pub station_id: String,
    /// Station endpoints, `host:port`, in preference order.
    pub endpoints: Vec<String>,
}

/// Parses an invitation (`lull://invite?…` or the station's web address
/// with the invitation in its fragment), e.g. to validate a scanned QR
/// code before pairing.
#[uniffi::export]
pub fn parse_invitation(input: String) -> Result<InvitationInfo, LullError> {
    let inv = lull_proto::Invitation::parse_any(&input).map_err(|e| LullError::Invalid {
        reason: e.to_string(),
    })?;
    let key = lull_crypto::PublicKey::from_bytes(inv.station_key);
    Ok(InvitationInfo {
        name: inv.name,
        station_id: key.device_id().as_str().to_owned(),
        endpoints: inv.endpoints.iter().map(ToString::to_string).collect(),
    })
}

/// Whether `input` is a usable manual address (`host:port`,
/// `[v6]:port`, `ws://…`, `wss://…`; FR-NET-2).
#[uniffi::export]
pub fn is_valid_route(input: String) -> bool {
    lull_core::listener::Route::parse(&input).is_some()
}

/// Version of the Lull core in these bindings.
#[uniffi::export]
pub fn core_version() -> String {
    lull_core::APP_NAME.to_owned()
}

/// Position of a level on the 0..1 meter and ring: linear in dB from
/// −90 dBFS (0) to −10 dBFS (1), clamped (UI-1).
#[uniffi::export]
pub fn level_fraction(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs + 90.0) / 80.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_and_levels() {
        let q = qr_code("lull://invite?x".into()).unwrap();
        assert_eq!(q.modules.len(), (q.size * q.size) as usize);
        assert!(q.modules.contains(&1));
        assert_eq!(level_fraction(-90.0), 0.0);
        assert_eq!(level_fraction(-10.0), 1.0);
        assert_eq!(level_fraction(-50.0), 0.5);
        assert!(is_valid_route("10.0.2.2:47100".into()));
        assert!(!is_valid_route("nope".into()));
        assert!(parse_device_id("garbage".into()).is_err());
    }
}
