use std::net::SocketAddr;
use tonic::transport::Server;

mod pb;
mod service;

// TODO(graceful shutdown): when the manager gets SIGTERM/SIGINT, stop the
// postgres it owns before exiting. Currently a stopped manager orphans postgres.
// Plan:
//  1. Share state with main: Arc<Mutex<Option<RunningInstance>>>, one clone in
//     the service, one kept in main.
//  2. Shutdown future: select! over tokio::signal::ctrl_c() and SIGTERM.
//  3. Replace .serve(addr) with .serve_with_shutdown(addr, shutdown_future).
//  4. After it returns: lock the state; if Some, run `pg_ctl stop -D <data_dir>
//     -m fast`, then child.wait().await, then set the slot to None.
//  5. Test guard: send SIGTERM to the manager instead of calling pg_ctl directly
//     (start_kill sends SIGKILL, which the manager can't catch).

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