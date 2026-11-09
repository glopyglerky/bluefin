//! Ephemeral foreground adapter. No installation, credentials or discovery.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: peer https://origin /Videos/item-id/"
    );
    let origin = bluefin_iroh_transport::Origin::new(&args[1], &args[2])?;
    let ep = bluefin_iroh_transport::endpoint().await?;
    println!("peerId={} addresses={:?}", ep.id(), ep.addr().addrs);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(());
    });
    bluefin_iroh_transport::run_adapter(ep, origin, rx).await;
    Ok(())
}
