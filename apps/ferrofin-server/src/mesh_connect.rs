//! Startup composition for the experimental pinned PQ mesh listener.
use std::{fs::File,io::Read as _,net::SocketAddr,path::PathBuf};
use anyhow::{Context as _,Result,ensure};
use bluefin_mesh_connect::native::{DeviceIdentity,PqcEndpoint};
use serde::Deserialize;
use std::future::IntoFuture as _;

#[derive(Clone,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// Existing owner-only directory; the reused identity store enforces
    /// no symlinks, 0700 parent, 0600 file and one live process lease.
    pub secret_seed_file:PathBuf,
    pub bind_address:SocketAddr,
    #[serde(default)]
    pub relay_urls:Vec<String>,
    /// Private relay hook. Absent by default; never mounted on the account API.
    #[serde(default)]
    pub relay_callout:Option<CalloutConfig>,
    /// Optional separately hosted, browser-trusted HTTPS application page.
    /// This never changes the account API or WebAuthn relying-party origin.
    #[serde(default)]
    pub web_launch_url:Option<String>,
    /// The public pure-Rust PQ provider is explicitly under qualification.
    pub allow_experimental_pqc:bool,
}
#[derive(Clone,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalloutConfig {
    pub bind_address:SocketAddr,
    pub relay_url:String,
}

struct CalloutRuntime {
    state:std::sync::Arc<ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout>,
    task:tokio::task::JoinHandle<()>,
}
impl Drop for CalloutRuntime {
    fn drop(&mut self){self.task.abort();}
}
impl CalloutRuntime {
    async fn start(config:&CalloutConfig,endpoint:[u8;32])->Result<Self>{
        ensure!(config.bind_address.ip().is_loopback() && config.bind_address.port()!=0,
            "relay callout requires a fixed loopback address");
        // Runtime injection only. Never include the value or parser errors in diagnostics.
        let token=std::env::var("BLUEFIN_RELAY_CALLOUT_BEARER")
            .map_err(|_|anyhow::anyhow!("relay callout runtime bearer unavailable"))?;
        let state=ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout::new(&config.relay_url,&token)
            .map_err(|_|anyhow::anyhow!("invalid relay callout audience or credential"))?;
        state.admissions.lock().await.admit_listener(endpoint,std::time::Instant::now())
            .map_err(|_|anyhow::anyhow!("listener admission refused"))?;
        let listener=tokio::net::TcpListener::bind(config.bind_address).await
            .context("private relay callout bind failed")?;
        let owner=state.clone();
        let serving=state.clone();
        let task=tokio::spawn(async move {
            let renewal=async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    if owner.admissions.lock().await.admit_listener(endpoint,std::time::Instant::now()).is_err(){break;}
                }
            };
            tokio::select! {
                _=axum::serve(listener,serving.router()).into_future()=>{},
                _=renewal=>{},
            }
            owner.admissions.lock().await.clear();
        });
        Ok(Self{state,task})
    }
    async fn stop(&self){
        self.task.abort();
        self.state.admissions.lock().await.clear();
    }
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
        if let Some(url)=&value.web_launch_url {
            bluefin_mesh_connect::launch::WebLaunchBase::parse(url)
                .context("mesh web launch must be a credential-free HTTPS page URL")?;
        }
        ensure!(!value.bind_address.ip().is_multicast(),"invalid mesh bind address");
        Ok(Some(value))
    }
}
pub(crate) struct Bound {
    endpoint:PqcEndpoint,
    // Reuse the existing hardened identity owner. Hold its exclusive file
    // lease until all network tasks have joined during shutdown.
    _identity_lease:File,
    callout:Option<CalloutRuntime>,
}
impl Bound {
    pub async fn bind(config:&Config)->Result<Self>{
        ensure!(config.allow_experimental_pqc,"experimental PQ provider is not enabled");
        let (seed,lease)=bluefin_iroh_server::open_private_identity_seed(&config.secret_seed_file)?;
        let identity=DeviceIdentity::from_seed(seed).context("mesh identity unavailable")?;
        let callout=if let Some(hook)=&config.relay_callout {
            ensure!(config.relay_urls.len()==1 && config.relay_urls[0]==hook.relay_url,
                "callout audience must exactly match the single configured relay");
            Some(CalloutRuntime::start(hook,identity.listener_endpoint_id()?).await?)
        }else{None};
        let endpoint=match PqcEndpoint::server(&identity,config.bind_address,config.relay_urls.clone(),
            ferrofin_api::handlers::bluefin_mesh_accounts::SERVICE).await {
            Ok(endpoint)=>endpoint,
            Err(error)=>{
                if let Some(hook)=&callout {hook.stop().await;}
                return Err(error).context("mesh listener unavailable");
            }
        };
        Ok(Self{endpoint,_identity_lease:lease,callout})
    }
    pub fn address(&self)->Result<bluefin_mesh_connect::ServerAddress>{
        self.endpoint.server_address().context("mesh address unavailable")
    }
    pub async fn close(&self){
        if let Some(hook)=&self.callout {hook.stop().await;}
        self.endpoint.close().await;
    }
    pub async fn run(self,state:ferrofin_api::AppState,app:axum::Router,stop:tokio::sync::watch::Receiver<bool>)
        ->Result<ferrofin_api::mesh_transport::Receipt>{
        let result=ferrofin_api::mesh_transport::run(self.endpoint.clone(),state,app,stop).await.map_err(Into::into);
        self.close().await;
        result
    }
}
