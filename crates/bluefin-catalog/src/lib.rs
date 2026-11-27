//! Portable catalog identities and source-backed regional access offers.
//!
//! Default features require only core and alloc. Credentialed HTTP is an optional host feature.
//! Empty offers must always be interpreted with the source status and coverage.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(any(feature = "http", test))]
extern crate std;

mod model;
pub mod video;
pub mod tmdb_links;
pub mod musicbrainz;
#[cfg(feature = "http")]
pub mod music_http;
pub use model::*;

#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "http")]
pub use http::{CatalogClient, ClientError, Credentials, MovieOfTheNightCredential, TmdbCredential};
