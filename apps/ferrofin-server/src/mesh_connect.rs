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
    state: std::sync::Arc<ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout>,
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for CalloutRuntime {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Ok(mut admissions) = self.state.admissions.try_lock() {
            admissions.clear();
        }
        if let Some(task) = self.task.get_mut().as_ref() {
            task.abort();
        }
    }
}

impl CalloutRuntime {
    async fn start(
        config: &CalloutConfig,
        endpoint: [u8; 32],
        identity_lease: std::sync::Arc<File>,
    ) -> Result<Self> {
        ensure!(
            config.bind_address.ip().is_loopback() && config.bind_address.port() != 0,
            "relay callout requires a fixed loopback address"
        );
        // Runtime injection only. Never include the value or parser errors in diagnostics.
        let token = std::env::var("BLUEFIN_RELAY_CALLOUT_BEARER")
            .map_err(|_| anyhow::anyhow!("relay callout runtime bearer unavailable"))?;
        let state = ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout::new(
            &config.relay_url,
            &token,
        )
        .map_err(|_| anyhow::anyhow!("invalid relay callout audience or credential"))?;
        state
            .admissions
            .lock()
            .await
            .admit_listener(endpoint, std::time::Instant::now())
            .map_err(|_| anyhow::anyhow!("listener admission refused"))?;
        let listener = tokio::net::TcpListener::bind(config.bind_address)
            .await
            .context("private relay callout bind failed")?;
        let router = std::sync::Arc::clone(&state).router();
        Ok(Self::serve(listener, state, endpoint, identity_lease, router))
    }

    fn serve(
        listener: tokio::net::TcpListener,
        state: std::sync::Arc<ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout>,
        endpoint: [u8; 32],
        identity_lease: std::sync::Arc<File>,
        router: axum::Router,
    ) -> Self {
        let owner = std::sync::Arc::clone(&state);
        let (shutdown, mut signal) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            // Keep the lease even if this future is aborted before its first poll.
            // Accepted connection services and the shutdown callback also retain it
            // until their actual futures are dropped, including on the Drop path.
            let lease = identity_lease;
            let shutdown_lease = std::sync::Arc::clone(&lease);
            let shutdown_signal = async move {
                let _identity_lease = shutdown_lease;
                loop {
                    if *signal.borrow() {
                        break;
                    }
                    tokio::select! {
                        biased;
                        changed = signal.changed() => {
                            if changed.is_err() || *signal.borrow() {
                                break;
                            }
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                            let mut admissions = owner.admissions.lock().await;
                            if *signal.borrow()
                                || admissions.admit_listener(endpoint, std::time::Instant::now()).is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                owner.admissions.lock().await.clear();
            };
            let router = router.layer(axum::Extension(std::sync::Arc::clone(&lease)));
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal)
                .await;
        });
        Self {
            state,
            shutdown,
            task: tokio::sync::Mutex::new(Some(task)),
        }
    }

    async fn stop(&self) {
        self.shutdown.send_replace(true);
        let mut task = self.task.lock().await;
        if let Some(join) = task.as_mut() {
            // Await the stored handle without taking it: cancelling this close
            // must leave ownership available to a later close or synchronous Drop.
            let _ = join.await;
            *task = None;
        }
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
    _identity_lease:std::sync::Arc<File>,
    callout:Option<CalloutRuntime>,
}
impl Bound {
    pub async fn bind(config:&Config)->Result<Self>{
        ensure!(config.allow_experimental_pqc,"experimental PQ provider is not enabled");
        let (seed,lease)=bluefin_iroh_server::open_private_identity_seed(&config.secret_seed_file)?;
        let lease=std::sync::Arc::new(lease);
        let identity=DeviceIdentity::from_seed(seed).context("mesh identity unavailable")?;
        let callout=if let Some(hook)=&config.relay_callout {
            ensure!(config.relay_urls.len()==1 && config.relay_urls[0]==hook.relay_url,
                "callout audience must exactly match the single configured relay");
            Some(CalloutRuntime::start(hook,identity.listener_endpoint_id()?,std::sync::Arc::clone(&lease)).await?)
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
    use tokio::sync::Notify;

    const ENDPOINT: [u8; 32] = [1; 32];

    async fn fixture(
        router: axum::Router,
    ) -> (CalloutRuntime, tempfile::TempDir, PathBuf, SocketAddr) {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().canonicalize().unwrap().join("identity");
        let (_, lease) = bluefin_iroh_server::open_private_identity_seed(&path).unwrap();
        let state = ferrofin_api::handlers::bluefin_mesh_relay::RelayCallout::new(
            "https://relay.example/",
            &"ab".repeat(32),
        )
        .unwrap();
        state
            .admissions
            .lock()
            .await
            .admit_listener(ENDPOINT, std::time::Instant::now())
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let runtime = CalloutRuntime::serve(listener, state, ENDPOINT, Arc::new(lease), router);
        (runtime, root, path, address)
    }

    fn held_router(entered: Arc<Notify>, release: Arc<Notify>) -> axum::Router {
        axum::Router::new().route(
            "/held",
            axum::routing::get(move || {
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                async move {
                    entered.notify_one();
                    release.notified().await;
                    "finished"
                }
            }),
        )
    }

    async fn wait_for_lease(path: &std::path::Path) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok((_, lease)) = bluefin_iroh_server::open_private_identity_seed(path) {
                    drop(lease);
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("callout retained identity ownership after its tasks exited");
    }

    #[tokio::test]
    async fn callout_close_joins_connections_and_can_resume_after_cancellation() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (runtime, _root, path, address) =
            fixture(held_router(Arc::clone(&entered), Arc::clone(&release))).await;
        let request = tokio::spawn(async move {
            reqwest::get(format!("http://{address}/held"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let mut close = Box::pin(runtime.stop());
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut close).await.is_err());
        drop(close);
        assert!(bluefin_iroh_server::open_private_identity_seed(&path).is_err());
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), runtime.stop()).await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), request).await.unwrap().unwrap(),
            "finished"
        );
        assert!(!runtime.state.admissions.lock().await.allows(&ENDPOINT, std::time::Instant::now()));
        assert!(bluefin_iroh_server::open_private_identity_seed(&path).is_ok());
        assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn callout_drop_retains_lease_until_an_unpolled_task_is_destroyed() {
        let (runtime, _root, path, address) = fixture(axum::Router::new()).await;
        drop(runtime);
        // abort only schedules cancellation; this task has not yielded yet.
        assert!(bluefin_iroh_server::open_private_identity_seed(&path).is_err());
        assert!(std::net::TcpListener::bind(address).is_err());
        wait_for_lease(&path).await;
        assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    }

    #[tokio::test]
    async fn callout_drop_keeps_identity_owned_by_an_accepted_connection() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (runtime, _root, path, address) =
            fixture(held_router(Arc::clone(&entered), Arc::clone(&release))).await;
        let request = tokio::spawn(async move {
            reqwest::get(format!("http://{address}/held"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        drop(runtime);
        tokio::task::yield_now().await;
        assert!(bluefin_iroh_server::open_private_identity_seed(&path).is_err());
        release.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), request).await.unwrap().unwrap(),
            "finished"
        );
        wait_for_lease(&path).await;
        assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    }
}
