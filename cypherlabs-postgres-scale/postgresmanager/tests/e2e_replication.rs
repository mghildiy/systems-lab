mod pb {
    tonic::include_proto!("cypherlabs.postgres.scale.postgresmanager");
}

use tempfile::{tempdir, tempfile};
use pb::postgres_manager_client::PostgresManagerClient;
use pb::{CreateReplicationSlotRequest, StartPrimaryRequest};

struct TestEnv {
    server_process: tokio::process::Child,
    data_dir: String,
    temp_dir: tempfile::TempDir,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        terminate_postgres(&self.data_dir);
        let _ = self.server_process.start_kill();
    }
}

async fn start_server() -> (tokio::process::Child, u16) {
    //let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    //let port = listener.local_addr().unwrap().port();
    //drop(listener);
    let port = free_port();
    println!("Assigned LISTEN_PORT: {}", port);
    let binary_path = env!("CARGO_BIN_EXE_postgresmanager");
    let server_process = tokio::process::Command::new(binary_path)
        .env("LISTEN_PORT", port.to_string())
        .spawn()
        .expect("failed to start gRPC server");
    (server_process, port)
}

async fn postgresmanager_connect_with_retry(addr: &str) -> Result<PostgresManagerClient<tonic::transport::Channel>,
        tonic::transport::Error> {
    let max_retries = 20;
    //let mut last_error = None;
    let mut attempts = 0;
    loop {
        match PostgresManagerClient::connect(addr.to_string()).await {
            Ok(client) => return Ok(client),
            Err(e) if attempts >= max_retries => return Err(e),
            Err(_) => {
                attempts += 1;
                // TODO jitter?
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    /*for _ in 0..max_retries {
        match PostgresManagerClient::connect(addr.to_string()).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            },
        }
    }

    panic!("Failed to connect to gRPC server after {} attempts: {:?}", max_retries, last_error);*/
}

async fn postgres_connect_with_retry(conn_string: &str) -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let max_attempts = 20;
    let mut attempts = 0;
    //let mut last_error = None;
    loop {
        match tokio_postgres::connect(conn_string, tokio_postgres::NoTls).await {
            Ok((client, connection)) => {
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Connection error: {}", e);
                    }
                });
                return Ok(client);
            }
            Err(e) if attempts >= max_attempts => return Err(e),
            Err(_) => {
                attempts += 1;
                // TODO jitter?
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

fn postgres_connection_string(user: &str, password: &str, host: &str, port: u32, database: &str) -> String {
    format!(
        "postgresql://{}:{}@{}:{}/{}",
        user, password, host, port, database
    )
}

fn kill_process_on_port(port: u16) {
    let output = std::process::Command::new("lsof")
        .arg("-ti")
        .arg(format!(":{}", port))
        .output()
        .unwrap();
    let pid_str = String::from_utf8_lossy(&output.stdout);
    for pid in pid_str.lines() {
        let _ = std::process::Command::new("kill").arg(pid).output();
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn terminate_postgres(data_dir: &str) {
    let _ = std::process::Command::new("pg_ctl")
        .args(["stop", "-D", data_dir, "-m", "immediate"])
        .output();
}

#[tokio::test]
async fn test_end_to_end_replication() {
    // launch gRPC server, and create test env for clean up during panic
    let (server_process, port) = start_server().await;
    let temp_dir = tempdir().expect("failed to create temp directory");
    let data_dir = temp_dir
        .path()
        .to_str()
        .expect("temp path is not valid UTF-8")
        .to_string();
    let env = TestEnv {
        server_process,
        data_dir,
        temp_dir,
    };

    // create gRPC client
    let channel_address = format!("http://127.0.0.1:{}", port);
    let mut postgresmanager = postgresmanager_connect_with_retry(&channel_address)
                                        .await
                                        .expect("failed to connect to postgres manager");
    // start primary
    let primary_port = free_port();
    let start_primary_request = StartPrimaryRequest {
        data_dir: env.data_dir.clone(),
        port: primary_port.into()
    };
    postgresmanager
        .start_primary(start_primary_request)
        .await
        .expect("Starting primary failed");

    // insert data in primary
    let usr = std::env::var("USER").unwrap_or_default();
    let db_connection_string = postgres_connection_string(&usr, "", "localhost",
                                                          primary_port.into(), "postgres");
    let postgres_client= postgres_connect_with_retry(&db_connection_string)
        .await
        .expect("failed to connect to postgres");
    let table_count = postgres_client
        .execute("CREATE TABLE test_table (id int, note text)", &[])
        .await
        .expect("failed to create table");
    let inserted_rows_count  = postgres_client
        .execute("INSERT INTO test_table VALUES (1, 'before replica')", &[])
        .await
        .expect("failed to insert data");
    assert_eq!(inserted_rows_count, 1, "Wrong number of rows inserted in table");

    // create replication slot
    let slot_name = "test_replication_slot";
    let create_replication_slot_response = postgresmanager
        .create_replication_slot(CreateReplicationSlotRequest {
        slot_name: slot_name.to_string()
    })
    .await
    .expect("failed to create replication slot");
    let lsn = &create_replication_slot_response.get_ref().lsn;
    assert!(!lsn.is_empty(), "Empty LSN not allowed");
    let lsn_parts: Vec<_> = lsn.split("/").collect();
    assert_eq!(lsn_parts.len(), 2, "LSN must have 2 parts");
    let hi = u64::from_str_radix(lsn_parts[0], 16).expect("bad hi in LSN");
    let lo = u64::from_str_radix(lsn_parts[1], 16).expect("bad lo in LSN");
    let lsn_value = (hi << 32) | lo;
    assert!(lsn_value > 0, "LSN should be a real WAL position");
    let restart_lsn = postgres_client
        .query_one("SELECT restart_lsn::text FROM pg_replication_slots WHERE slot_name = $1", &[&slot_name])
        .await
        .expect("replication slot query failed");
    let stored_lsn: String = restart_lsn.get(0);
    assert_eq!(&stored_lsn, lsn, "LSN mismatch");
}