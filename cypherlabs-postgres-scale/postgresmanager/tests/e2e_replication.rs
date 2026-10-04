mod pb {
    tonic::include_proto!("cypherlabs.postgres.scale.postgresmanager");
}

use tonic::Status;
use pb::postgres_manager_client::PostgresManagerClient;
use pb::postgres_manager_server::PostgresManagerServer;
use pb::{StartPrimaryRequest, CreateReplicationSlotRequest, StartReplicaRequest};

async fn start_server() -> (tokio::process::Child, u16) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    println!("Assigned LISTEN_PORT: {}", port);
    let binary_path = env!("CARGO_BIN_EXE_postgresmanager");
    let server_process = tokio::process::Command::new(binary_path)
        .env("LISTEN_PORT", port.to_string())
        .spawn()
        .expect("failed to start gRPC server");
    (server_process, port)
}

async fn retry_connect(addr: &str) -> PostgresManagerClient<tonic::transport::Channel> {
    let max_retries = 20;
    let mut last_error = None;
    for _ in 0..max_retries {
        match PostgresManagerClient::connect(addr.to_string()).await {
            Ok(client) => return client,
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            },
        }
    }

    panic!("Failed to connect to gRPC server after {} attempts: {:?}", max_retries, last_error);
}

async fn postgres_retry_connect(conn_string: &str) -> tokio_postgres::Client {
    let max_retries = 20;
    let mut last_error = None;
    for _ in 0..max_retries {
        match tokio_postgres::connect(conn_string, tokio_postgres::NoTls).await {
            Ok((client, connection)) => {
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Connection error: {}", e);
                    }
                });
                return client;
            }
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    panic!("Failed to connect to postgres after {} attempts: {:?}", max_retries, last_error);
}

fn postgres_connection_string(user: &str, password: &str, host: &str, port: u32, database: &str) -> String {
    format!(
        "postgresql://{}:{}@{}:{}/{}",
        user, password, host, port, database
    )
}

#[tokio::test]
async fn test_end_to_end_replication() {
    // launch gRPC server
    let (mut server_process, port) = start_server().await;

    // create gRPC client
    let channel_address = format!("http://127.0.0.1:{}", port);
    let mut postgresmanager_client = retry_connect(&channel_address).await;

    // start primary
    let response = postgresmanager_client.start_primary(StartPrimaryRequest {
        data_dir: "/tmp/test-e2e-primary".to_string(),
        port: 5500,
    })
    .await
    .unwrap();
    println!("Start primary response: {}", response.get_ref().pid);

    // insert data in primary
    let usr = std::env::var("USER").unwrap_or_default();
    let db_connection_string = postgres_connection_string(&usr, "",
                                                          "localhost", 5500, "postgres");
    /*let (client, connection) = tokio_postgres::connect(&db_connection_string, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
    if let Err(e) = connection.await {
        eprintln!("Connection error: {}", e);
    }
    });*/
    let client= postgres_retry_connect(&db_connection_string).await;
    client
        .query("CREATE TABLE test_table (id int, note text)", &[])
        .await
        .unwrap();
    client
        .query("INSERT INTO test_table VALUES (1, 'before replica')", &[])
        .await
        .unwrap();

    // create replication slot
    let slot_name = "test_replication_slot";
    let create_replication_slot_response = postgresmanager_client
        .create_replication_slot(CreateReplicationSlotRequest {
        slot_name: slot_name.to_string()
    })
    .await
    .unwrap();
    let lsn = &create_replication_slot_response.get_ref().lsn;
    println!("LSN for replication slot {}: {}", slot_name,lsn);

    // clean up
    server_process.kill();
}