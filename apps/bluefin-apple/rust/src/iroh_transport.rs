//! RAII owner for a pinned foreground tunnel. Capability URLs never leave the proxy.
//!
//! Playback requests optionally include `"iroh":{"peerId":"<pinned endpoint id>",
//! "peerAddress":"<numeric IP:port>"}`. No automatic discovery or HTTPS fallback.
//! Viewer auth comes from the usual token/deviceId fields, never a second credential.
//! Canonical server/media URLs remain HTTPS and HLS validation remains item scoped.
//! Only upstream fetch URLs change; callers receive the existing core loopback URL.
//! Call stopPlaybackProxy on cancellation, replacement, failed player setup or exit.
//! Proxy shutdown drains jobs before dropping this tunnel owner.
use super::{err, Api, Result};
use bluefin_iroh_transport::ClientConfig;
use url::Url;
pub(super) struct Tunnel {
    id: Option<u64>,
    base: String,
    mesh: bool,
    origin: Option<Url>,
    #[cfg(test)]
    drain: Option<fn(u64) -> Result<()>>,
}
impl Tunnel {
    pub(super) fn from_request(api: &Api<'_>) -> Result<Option<Self>> {
        if api.v["mesh"].is_object() {
            let config=super::mesh_connection::Config::from_request(api.v)?;
            let origin=config.origin.clone();
            let (id,base)=super::mesh_proxy::start(config)?;
            return Ok(Some(Self {id:Some(id),base,mesh:true,origin:Some(origin),
                #[cfg(test)] drain:None}));
        }
        let Some(config) = api.v.get("iroh").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let object = config
            .as_object()
            .ok_or_else(|| err("Invalid iroh configuration"))?;
        if object.len() != 2
            || !object.contains_key("peerId")
            || !object.contains_key("peerAddress")
        {
            return Err(err("Iroh requires peerId and peerAddress only"));
        }
        if api.base.scheme() != "https" {
            return Err(err("Iroh requires canonical HTTPS media origin"));
        }
        let config = ClientConfig {
            peer_id: super::text(config, "peerId")?.into(),
            peer_address: super::text(config, "peerAddress")?
                .parse()
                .map_err(|_| err("Invalid Iroh peerAddress"))?,
            authorization: api.auth(true)?,
        };
        let (id, base) =
            bluefin_iroh_transport::start(config).map_err(|_| err("Iroh tunnel unavailable"))?;
        Ok(Some(Self {
            id: Some(id),
            base,
            mesh: false,
            origin: None,
            #[cfg(test)]
            drain: None,
        }))
    }
    #[cfg(test)]
    pub(super) fn failed_drain() -> Self {
        Self {
            id: Some(0),
            base: "http://127.0.0.1:1".into(),
            mesh: false,
            origin: None,
            drain: Some(|_| Err(err("Iroh tunnel cleanup unconfirmed"))),
        }
    }
    pub(super) fn fetch_url(&self, canonical: &Url) -> Result<Url> {
        if self.origin.as_ref().is_some_and(|origin|origin.origin()!=canonical.origin()) {
            return Err(err("This media resource is outside the paired server."));
        }
        Url::parse(&format!(
            "{}{}{}",
            self.base,
            canonical.path(),
            canonical
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        ))
        .map_err(|_| err("Invalid Iroh media route"))
    }
}
impl Tunnel {
    pub(super) fn stop(&mut self) -> Result<()> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        #[cfg(test)]
        if let Some(drain) = self.drain {
            return drain(id);
        }
        if self.mesh {return super::mesh_proxy::stop(id);}
        bluefin_iroh_transport::stop_with_receipt(id)
            .map_err(|_| err("Iroh tunnel cleanup unconfirmed"))
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        // Startup failure and unwind fallback. Only explicit stop produces a receipt.
        let _ = self.stop();
    }
}
