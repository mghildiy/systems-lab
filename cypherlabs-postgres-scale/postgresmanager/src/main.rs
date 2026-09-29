use std::net::SocketAddr;
use tonic::transport::Server;

mod pb;
mod service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pg_manager = service::PostgresManagerService::new();
    let socker_addr: SocketAddr = "127.0.0.1:50051".parse::<SocketAddr>().unwrap();
    Server::builder()
        .add_service(pb::postgres_manager_server::PostgresManagerServer::new(pg_manager))
        .serve(socker_addr)
        .await?;

    Ok(())
}