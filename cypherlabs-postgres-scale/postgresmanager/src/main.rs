use std::net::SocketAddr;
use tonic::transport::Server;

mod pb;
mod service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pg_manager = service::PostgresManagerService::new();
    let port = std::env::var("LISTEN_PORT").unwrap_or("50051".to_string());
    let socker_addr: SocketAddr = format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap();
    Server::builder()
        .add_service(pb::postgres_manager_server::PostgresManagerServer::new(pg_manager))
        .serve(socker_addr)
        .await?;

    Ok(())
}