use std::net::SocketAddr;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tonic::Status;
use tonic::transport::Server;
use crate::service::{PostgresManagerService, RunningInstance};

mod pb;
mod service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(Mutex::new(None));
    let service = PostgresManagerService::new(Arc::clone(&state));
    let port = std::env::var("LISTEN_PORT").unwrap_or("50051".to_string());
    let socket_addr: SocketAddr = format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap();
    Server::builder()
        .add_service(pb::postgres_manager_server::PostgresManagerServer::new(service))
        .serve_with_shutdown(socket_addr, shutdown_signal())
        .await?;

    println!("server stopped, running cleanup");
    let mut guard = state.lock().await;
    if let Some(mut ri) = guard.take() {
        cleanup(&mut ri).await;
    }

    Ok(())
}

async fn cleanup(ri: &mut RunningInstance) {
    match Command::new("pg_ctl")
        .args(&["stop", "-D", &ri.data_dir, "-m", "fast"])
        .output().await {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!("pg_ctl stop failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr));
            let _ = ri.child.kill().await;
        }
        Err(e) => eprintln!("could not run pg_ctl stop: {e}"),
    }
    if let(Err(e)) = ri.child.wait().await {
        eprintln!("waiting for postgres to exit failed: {e}");
        let _ = ri.child.kill().await;
    }
}

async fn shutdown_signal() {
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    )
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
    }
}