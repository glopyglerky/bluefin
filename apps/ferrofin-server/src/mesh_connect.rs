//! Startup composition for the experimental pinned PQ mesh listener.
use std::{fs::File,io::Read as _,net::SocketAddr,path::PathBuf};
use anyhow::{Context as _,Result,ensure};
use bluefin_mesh_connect::native::{DeviceIdentity,PqcEndpoint};
use serde::Deserialize;

#[derive(Clone,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// Existing owner-only directory; the reused identity store enforces
    /// no symlinks, 0700 parent, 0600 file and one live process lease.
    pub secret_seed_file:PathBuf,
    pub bind_address:SocketAddr,
    #[serde(default)]
    pub relay_urls:Vec<String>,
    /// The public pure-Rust PQ provider is explicitly under qualification.
    pub allow_experimental_pqc:bool,
}
impl Config {
    pub fn from_env()->Result<Option<Self>>{
        let Some(path)=std::env::var_os("BLUEFIN_MESH_CONNECT_CONFIG")else{return Ok(None)};
        let file=File::open(path).context("mesh connection configuration unavailable")?;
        ensure!(file.metadata()?.is_file(),"regular mesh configuration required");
        let mut bytes=Vec::new();file.take(16*1024+1).read_to_end(&mut bytes)?;
        ensure!(bytes.len()<=16*1024,"mesh configuration is oversized");
        let value:Self=serde_json::from_slice(&bytes).context("invalid mesh connection configuration")?;
        ensure!(value.allow_experimental_pqc,"mesh PQ provider requires explicit experimental opt-in");
        ensure!(value.relay_urls.len()<=2,"at most two approved mesh relays");
        ensure!(!value.bind_address.ip().is_multicast(),"invalid mesh bind address");
        Ok(Some(value))
    }
}
pub(crate) struct Bound {
    endpoint:PqcEndpoint,
    // Reuse the existing hardened identity owner. Hold its exclusive file
    // lease until all network tasks have joined during shutdown.
    _identity_lease:File,
}
impl Bound {
    pub async fn bind(config:&Config)->Result<Self>{
        ensure!(config.allow_experimental_pqc,"experimental PQ provider is not enabled");
        let (seed,lease)=bluefin_iroh_server::open_private_identity_seed(&config.secret_seed_file)?;
        let identity=DeviceIdentity::from_seed(seed).context("mesh identity unavailable")?;
        let endpoint=PqcEndpoint::server(&identity,config.bind_address,config.relay_urls.clone(),
            ferrofin_api::handlers::bluefin_mesh_accounts::SERVICE).await.context("mesh listener unavailable")?;
        Ok(Self{endpoint,_identity_lease:lease})
    }
    pub fn address(&self)->Result<bluefin_mesh_connect::ServerAddress>{
        self.endpoint.server_address().context("mesh address unavailable")
    }
    pub async fn close(&self){self.endpoint.close().await;}
    pub async fn run(self,state:ferrofin_api::AppState,app:axum::Router,stop:tokio::sync::watch::Receiver<bool>)
        ->Result<ferrofin_api::mesh_transport::Receipt>{
        ferrofin_api::mesh_transport::run(self.endpoint.clone(),state,app,stop).await.map_err(Into::into)
    }
}
