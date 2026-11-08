//! Compilation-only upstream tvOS reproduction. Do not run on a live network.
pub async fn endpoint() -> Result<iroh::Endpoint, Box<dyn std::error::Error + Send + Sync>> {
    Ok(iroh::Endpoint::builder().relay_mode(iroh::RelayMode::Disabled).bind().await?)
}
