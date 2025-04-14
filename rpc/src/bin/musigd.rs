use rpc::wallet::WalletServiceImpl;
use rpc::server::{MusigImpl, MusigServer, WalletImpl, WalletServer};
use std::sync::Arc;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    let musig = MusigImpl::default();
    let wallet = WalletImpl { wallet_service: Arc::new(WalletServiceImpl::new()) };
    wallet.wallet_service.clone().spawn_connection();

    Server::builder()
        .add_service(MusigServer::new(musig))
        .add_service(WalletServer::new(wallet))
        .serve(addr)
        .await?;

    Ok(())
}
