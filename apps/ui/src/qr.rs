// SPDX-License-Identifier: AGPL-3.0-or-later
//! QR codes for invitations (product.md FR-PAIR-1), rendered as one SVG
//! path so they scale crisply and need no image files.

use qrcode::{EcLevel, QrCode};

/// A QR code as SVG geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrSvg {
    /// Side length in modules, including the quiet zone.
    pub size: usize,
    /// Path data of the dark modules (`M x y h n v1 h-n z` runs per row).
    pub path: String,
}

/// Quiet zone in modules (ISO/IEC 18004 asks for 4; the `.qr` frame adds
/// white padding, so 2 suffice).
const QUIET: usize = 2;

/// Encodes `text` with error correction level M.
pub fn qr_svg(text: &str) -> Option<QrSvg> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let w = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    let mut path = String::new();
    for y in 0..w {
        let mut x = 0;
        while x < w {
            if dark[y * w + x] {
                let start = x;
                while x < w && dark[y * w + x] {
                    x += 1;
                }
                if !path.is_empty() {
                    path.push(' ');
                }
                path.push_str(&format!(
                    "M{} {}h{}v1h-{}z",
                    start + QUIET,
                    y + QUIET,
                    x - start,
                    x - start
                ));
            } else {
                x += 1;
            }
        }
    }
    Some(QrSvg {
        size: w + 2 * QUIET,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_an_invitation_into_runs() {
        let inv = "lull://invite?v=1&s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&t=AAAAAAAAAAAAAAAAAAAAAA&n=Nursery&a=192.168.1.20%3A47100";
        let q = qr_svg(inv).unwrap();
        // Version ≥ 5 for this length; finder pattern at the top left.
        assert!(q.size >= 37 + 2 * QUIET);
        assert!(q.path.starts_with("M2 2h7v1h-7z"));
        // Every run is well-formed.
        assert!(q.path.ends_with('z'));
        assert!(
            q.path
                .split_terminator('z')
                .all(|r| r.trim_start().starts_with('M'))
        );
    }

    #[test]
    fn too_long_input_fails() {
        assert!(qr_svg(&"x".repeat(5000)).is_none());
    }
}
