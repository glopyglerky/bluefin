//! One invitation, shared by camera, image, clipboard and manual entry.
//!
//! An invitation locates an authenticated owner. It does not grant an account,
//! authorize a relay, or establish that it is still unused. Only the current
//! durable owner may consume it. Native connections require a pinned hybrid
//! post-quantum TLS session and fresh device key possession before application
//! data can be sent. No system VPN is used.
#![forbid(unsafe_code)]

mod capsule;
pub mod resource_grant;
mod error;
pub mod launch;
pub mod protocol;
pub mod discovery;
pub mod relay_admission;
#[cfg(feature = "images")]
mod qr;
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;
#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub mod browser;

pub use capsule::{ConnectInvite, ConnectInviteV1, InviteIntent, ServerAddress, MAX_CODE_BYTES};
pub use error::{Error, Result};
#[cfg(feature = "images")]
pub use qr::{QrTheme, decode_qr_image, decode_qr_image_at, qr_png, qr_svg, qr_png_for_web, qr_svg_for_web};
