use std::env;
use std::process::exit;
use tonic::Status;
use pgmanager_proto::postgres_manager_client::PostgresManagerClient;
use pgmanager_proto::{Role, StatusRequest};

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if let None = args.get(1) {
        println!("Postgres address is not provided, process exiting");
        return;
    }

    let address = args.get(1).unwrap();
    let mut pgmanager_client = match pgmanager_connect_with_retry(&address).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Failed to get postgres manager client with error: {}", error);
            exit(1);
        }
    };

    let response = pgmanager_client
                                                        .status(StatusRequest{})
                                                        .await;
    let server_response = match response {
        Ok(mut response) => response,
        Err(error) => {
            eprintln!("Failed to get postgres manager status with error: {}", error);
            exit(1);
        }
    };
    let status_response = server_response.into_inner();
    let pid = status_response.pid;
    let port = status_response.port;
    let role = Role::try_from(status_response.role).unwrap();
    let state = Status::try_from(status_response.state).unwrap();


}

async fn pgmanager_connect_with_retry(addr: &str) -> Result<PostgresManagerClient<tonic::transport::Channel>,
    tonic::transport::Error> {
    let max_retries = 20;
    let mut attempts = 0;
    loop {
        match PostgresManagerClient::connect(addr.to_string()).await {
            Ok(client) => return Ok(client),
            Err(e) if attempts >= max_retries => return Err(e),
            Err(_) => {
                attempts += 1;
                // TODO jitter
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}
