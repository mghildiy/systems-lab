mod pb {
    tonic::include_proto!("cypherlabs.postgres.scale.postgresmanager");
}

use tempfile::tempdir;
use tokio_postgres::{Client, Error, Row};
use tokio_postgres::types::ToSql;
use pb::postgres_manager_client::PostgresManagerClient;
use pb::{CreateReplicationSlotRequest, StartPrimaryRequest};
use crate::pb::StartReplicaRequest;

struct TestEnv {
    server_process: tokio::process::Child,
    data_dir: String,
    temp_dir: tempfile::TempDir,
    replica_data_dir: Option<String>,
    replica_temp_dir: Option<tempfile::TempDir>
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Some(dir) = &self.replica_data_dir {
            terminate_postgres(dir);
        }
        terminate_postgres(&self.data_dir);
        let _ = self.server_process.start_kill();
    }
}

async fn start_server() -> (tokio::process::Child, u16) {
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
}

async fn postgres_connect_with_retry(conn_string: &str) -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let max_attempts = 20;
    let mut attempts = 0;
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
    let mut env = TestEnv {
        server_process,
        data_dir,
        temp_dir,
        replica_data_dir: None,
        replica_temp_dir: None
    };

    // create gRPC client
    let channel_address = format!("http://127.0.0.1:{}", port);
    let mut postgresmanager = postgresmanager_connect_with_retry(&channel_address)
                                        .await
                                        .expect("failed to connect to postgres manager");
    // start primary
    let primary_port = free_port();
    println!("Primary port: {}", primary_port);
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
    let primary_connection_string = postgres_connection_string(&usr, "", "127.0.0.1",
                                                          primary_port.into(), "postgres");
    let primary_postgres_client= postgres_connect_with_retry(&primary_connection_string)
        .await
        .expect("failed to connect to primary");
    run_command(&primary_postgres_client, "CREATE TABLE test_table (id int, note text)",
                &[], "CREATE_TABLE_IN_PRIMARY").await;
    let inserted_rows_count: u64 = run_command(&primary_postgres_client,
                                               "INSERT INTO test_table VALUES (1, 'before replica')",
                                               &[], "INSERT_DATA_IN_TABLE_IN_PRIMARY").await;
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

    let lsn_primary_query = "SELECT restart_lsn::text FROM pg_replication_slots WHERE slot_name = $1";
    let replication_slot_lsn_record = run_query(&primary_postgres_client, lsn_primary_query,
                                 &[&slot_name], "GET_REPLICATION_SLOT_LSN").await;
    let stored_lsn: String = replication_slot_lsn_record.get(0);
    assert_eq!(&stored_lsn, lsn, "LSN mismatch");

    // launch replica
    let replica_temp_dir = tempdir().expect("failed to create temp directory");
    let replica_data_dir = replica_temp_dir
        .path()
        .to_str()
        .expect("temp path is not valid UTF-8")
        .to_string();
    let replica_port = free_port();
    println!("Replica port: {}", replica_port);
    let start_replica_request = StartReplicaRequest {
        data_dir: replica_data_dir.clone(),
        port: replica_port.into(),
        primary_host: "127.0.0.1".to_string(),
        primary_port: primary_port.into(),
        slot_name: slot_name.to_string()
    };
    env.replica_temp_dir = Some(replica_temp_dir);
    env.replica_data_dir = Some(replica_data_dir);


    let start_replica_response = postgresmanager
        .start_replica(start_replica_request)
        .await
        .expect("failed to start replica");
    let replica_connection_string = postgres_connection_string(&usr, "", "127.0.0.1",
                                                               replica_port.into(), "postgres");
    let replica_postgres_client= postgres_connect_with_retry(&replica_connection_string)
        .await
        .expect("failed to connect to replica");

    let replica_recovery_status_record = run_query(&replica_postgres_client,
                                            "SELECT pg_is_in_recovery()", &[],
                                            "CHECK_REPLICA_RECOVERY_STATUS").await;
    let is_recovery_mode_on: bool = replica_recovery_status_record.get(0);
    assert!(is_recovery_mode_on, "Recovery mode for replica must be TRUE");

    let replica_data_record = run_query(&replica_postgres_client,
                                        "SELECT note FROM test_table WHERE id = 1",
                                        &[], "CHECK_REPLICA_DATA").await;
    let actual_note: String = replica_data_record.get(0);
    assert_eq!(actual_note, "before replica", "note column value not as expected");

    let inserted_rows_count: u64 = run_command(&primary_postgres_client,
                                               "INSERT INTO test_table VALUES (2, 'after replica')",
                                               &[], "INSERT_DATA_IN_TABLE_IN_PRIMARY").await;
    assert_eq!(inserted_rows_count, 1, "Wrong number of rows inserted in table");

    let replica_data_record = run_query_for_timeperiod(&replica_postgres_client, "SELECT note FROM test_table WHERE id = 2",
                             &[], "CHECK_REPLICA_DATA").await;
    let actual_note: String = replica_data_record.get(0);
    assert_eq!(actual_note, "after replica", "note column value not as expected");
}

async fn run_query(client: &Client, query: &str, params: &[&(dyn ToSql + Sync)], context: &str)
        -> Row {
    match client
        .query_one(query, params)
        .await {
        Ok(row) => row,
        Err(e) => {
            panic!("failed to execute query {}, error: {}, message: {}", query, e, context);
        }
    }
}

async fn run_command(client: &Client, command: &str, params: &[&(dyn ToSql + Sync)], context: &str)
        -> u64 {
    match client
        .execute(command, params)
        .await {
        Ok(num) => num,
        Err(e) => {
            panic!("failed to execute command {}, error: {}, message: {}", command, e, context);
        }
    }
}

async fn run_query_for_timeperiod(client: &Client, query: &str, params: &[&(dyn ToSql + Sync)],
                                context: &str) -> Row {
    let start_time = std::time::Instant::now();
    let timeout = 5;
    loop {
        match client.query_opt(query, params).await {
            Ok(Some(row)) => return row,
            Ok(None) if std::time::Instant::now() >= start_time + std::time::Duration::from_secs(timeout)=> {
                panic!("No data found in replica after {} seconds", timeout);
            },
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            Err(e) => panic!("query failed: {query}, error: {e}, context: {context}")
        }
    }
}