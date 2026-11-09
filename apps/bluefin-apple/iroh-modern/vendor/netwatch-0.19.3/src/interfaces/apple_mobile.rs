//! Public getifaddrs enumeration for foreground direct-address clients.
//! Default gateway metadata is unavailable through the public mobile SDK.
pub(super) use super::netdev_impl::get_state;
use super::{DefaultRouteDetails, HomeRouter};
pub(super) async fn default_route() -> Option<DefaultRouteDetails> {
    None
}
pub(super) fn home_router() -> Option<HomeRouter> {
    None
}
