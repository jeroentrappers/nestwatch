// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera view (FR-CAM-1, FR-CAM-4): newest JPEG frame as an image, night
//! image toggle and camera switch.

use dioxus::prelude::*;

use super::common::Toggle;
use crate::backend::{BackendHandle, Ui};
use crate::model::{Tile, Video, VideoProblem};
use crate::strings as s;

/// The live view of one station.
#[component]
pub fn CameraView(tile: Tile) -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut night = use_signal(|| false);
    let mut front = use_signal(|| false);
    let frame = ui.frames.read().get(&tile.id).cloned();
    let streaming = matches!(tile.video, Video::Streaming { .. });
    let message = match &tile.video {
        Video::Starting | Video::Off => Some(s::CAMERA_STARTING),
        Video::Streaming { .. } if frame.is_none() => Some(s::CAMERA_STARTING),
        Video::Streaming { .. } => None,
        Video::Problem(VideoProblem::Disabled) => Some(s::CAMERA_DISABLED),
        Video::Problem(VideoProblem::Unavailable) => Some(s::CAMERA_UNAVAILABLE),
        Video::Problem(VideoProblem::Denied) => Some(s::CAMERA_DENIED),
        Video::Problem(VideoProblem::Stopped) => Some(s::CAMERA_STOPPED),
    };
    let info = match &tile.video {
        Video::Streaming { w, h, fps } => Some(s::frame_info(*w, *h, *fps)),
        _ => None,
    };
    let alt = s::camera_alt(&tile.name);
    let (b1, id1) = (backend.clone(), tile.id.clone());
    let (b2, id2) = (backend, tile.id.clone());
    rsx! {
        section { class: "stack-s camera", aria_label: "{s::camera_of(&tile.name)}",
            div { class: "video",
                if let Some(f) = frame.filter(|_| streaming) {
                    img { src: "{f.src}", alt: "{alt}", width: "{f.width}", height: "{f.height}" }
                } else {
                    div { class: "placeholder" }
                }
                if streaming {
                    span { class: "pill cam badge", "{s::CAMERA_LIVE}" }
                }
                if let Some(m) = message {
                    p { class: "video-message", role: "status", "{m}" }
                }
            }
            div { class: "row-between wrap",
                div { class: "row",
                    Toggle {
                        label: s::NIGHT_IMAGE.to_owned(),
                        pressed: *night.read(),
                        onclick: move |_| {
                            let n = !*night.peek();
                            night.set(n);
                            let _ = b1.set_video_options(&id1, *front.peek(), n);
                        },
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| {
                            let f = !*front.peek();
                            front.set(f);
                            let _ = b2.set_video_options(&id2, f, *night.peek());
                        },
                        "{s::SWITCH_CAMERA}"
                    }
                }
                if let Some(i) = info {
                    span { class: "caption", "{i}" }
                }
            }
        }
    }
}
