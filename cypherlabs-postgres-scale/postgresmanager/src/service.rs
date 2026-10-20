use crate::pb::{State, StatusRequest, StatusResponse, Role as PbRole};
use crate::pb::postgres_manager_server::PostgresManager;
use crate::pb::{CreateReplicationSlotRequest, CreateReplicationSlotResponse, StartPrimaryRequest, StartPrimaryResponse, StartReplicaRequest, StartReplicaResponse};
use crate::service::Role::{Primary, Replica};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tonic::{async_trait, Request, Response, Status};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Primary,
    Replica,
}

enum Found {
    Nothing,  // slot empty
    Alive{ role: Role, port: u32 },    // a record exists and its postgres is still running
    Stale,    // a record exists but its postgres has exited
}

struct RunningInstance {
    child: Child,
    port: u32,
    data_dir: String,
    role: Role
}

pub(crate) struct PostgresManagerService {
    running_instance: Mutex<Option<RunningInstance>>
}

impl PostgresManagerService {
    pub(crate) fn new() -> Self {
        let running_instance = Mutex::new(None);
        PostgresManagerService {
            running_instance
        }
    }
}

#[async_trait]
impl PostgresManager for PostgresManagerService {

    async fn start_primary(&self, request: Request<StartPrimaryRequest>)
            -> Result<Response<StartPrimaryResponse>, Status> {
        let mut guard = self.running_instance.lock().await;
        let state = check_existing(&mut *guard, "START_PRIMARY_CHECK_EXISTING_INSTANCE")?;
        match state {
            Found::Alive { role, port } => return Err(Status::failed_precondition(
                format!("already running as {:?} on port {}", role, port)
            )),
            Found::Stale => { *guard = None; }
            Found::Nothing => {}
        }

        let start_primary_request = request.get_ref();
        let data_dir = &start_primary_request.data_dir;
        let port = start_primary_request.port;

        let is_data_dir_initialized = Path::exists(PathBuf::from(data_dir).join("PG_VERSION").as_path());
        if !is_data_dir_initialized {
            let mut initdb_process = run_command("initdb",
                                                 &["-D", data_dir],
                                                 "EXECUTE_POSTGRES_INITDB_IN_PRIMNARY")?;
            if initdb_process.id().is_none() {
                return Err(Status::internal("PID not set"));
            };

            let exitstatus = initdb_process.wait().await
                .map_err(|e| to_internal_status("LAUNCH_PRIMARY_INITDB", e))?;
            if !exitstatus.success() {
                return Err(Status::internal(format!("DB initialization failed with error {}",
                                                    exitstatus)));
            }
        }

        // TODO: after a manager restart, state is empty even if a postgres is still running
        // on this data_dir (orphan from a crash). Detect via postmaster.pid (pid alive?)
        // and either adopt or refuse. Mostly prevented by graceful shutdown (see main.rs).

        let mut postgres_process = run_command("postgres",
                                               &["-D", data_dir, "-p", &port.to_string()],
                                               "START_PRIMARY_POSTGRES_SPAWN")?;

        let pid = match postgres_process.id() {
            Some(pid) => pid,
            None => return Err(Status::internal("postgres pid not available")),
        };

        // wait for postgres to be ready within a time period, else return
        if let Err(e) = wait_until_pg_ready(&mut postgres_process, data_dir, "TEST_READINESS_FOR_PRIMARY_PG")
            .await {
            let _ = postgres_process.kill().await;
            return Err(e);
        }

        *guard = Some(RunningInstance{port, child: postgres_process,
            data_dir: data_dir.to_string(), role: Primary});
        Ok(Response::new(StartPrimaryResponse { pid }))
    }

    async fn create_replication_slot(&self, request: Request<CreateReplicationSlotRequest>)
            -> Result<Response<CreateReplicationSlotResponse>, Status> {
        let create_replication_slot_request = request.get_ref();
        let guard = self.running_instance.lock().await;
        let port = match &*guard {
            Some(ri) => ri.port,
            None => return Err(Status::failed_precondition("Primary is not running; call StartPrimary first")),
        };
        drop(guard);

        let slot_name = &create_replication_slot_request.slot_name;

        let usr = std::env::var("USER").unwrap_or_default();
        let db_connection_string = postgres_connection_string(&usr, "",
         "localhost", port, "postgres");

        let (client, connection) = tokio_postgres::connect(&db_connection_string, tokio_postgres::NoTls)
            .await
            .map_err(|e| {
                eprintln!("{}", e);
                Status::internal("Failed to connect to Postgres")
            })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("Connection error: {}", e);
            }
        });

        // TODO(v0 gap): CreateReplicationSlot currently errors if a slot with this name
        // already exists (Postgres itself rejects the duplicate create). No idempotency
        // check is performed first (e.g., SELECT 1 FROM pg_replication_slots WHERE slot_name = $1).
        // Decision deferred, same as StartPrimary's analogous gap: should a repeat call
        // against an existing slot be (a) an error (current behavior, acceptable short-term),
        // or (b) idempotent success (detect existing slot, return its current LSN without
        // attempting to recreate)? Revisit once Operator's real retry semantics are known.

        let row = client
            .query_one("SELECT lsn::text FROM pg_create_physical_replication_slot($1, true)", &[&slot_name])
            .await
            .map_err(|e| {
                eprintln!("{}", e);
                Status::internal("Failed to create replication slot")
            })?;
        let lsn = row.get("lsn");

        Ok(Response::new(crate::pb::CreateReplicationSlotResponse { lsn }))
    }

    async fn start_replica(&self, request: Request<StartReplicaRequest>)
            -> Result<Response<StartReplicaResponse>, Status> {
        let mut guard = self.running_instance.lock().await;
        let state = check_existing(&mut *guard, "START_REPLICA_CHECK_EXISTING_INSTANCE")?;
        match state {
            Found::Alive { role, port } => return Err(Status::failed_precondition(
                format!("already running as {:?} on port {}", role, port)
            )),
            Found::Stale => { *guard = None; }
            Found::Nothing        => {}
        }

        let request = request.get_ref();
        let data_dir = &request.data_dir;
        let port = request.port;
        let primary_host = &request.primary_host;
        let primary_port = request.primary_port;
        let slot_name = &request.slot_name;
        let usr = std::env::var("USER").unwrap_or_default();

        let mut pg_basebackup_command = Command::new("pg_basebackup");
        let mut pg_basebackup_process = pg_basebackup_command
            .arg("-D")
            .arg(&data_dir)
            .arg("-h")
            .arg(primary_host)
            .arg("-p")
            .arg(primary_port.to_string())
            .arg("-U")
            .arg(usr)
            .arg("-S")
            .arg(slot_name)
            .arg("-R")
            .arg("-X")
            .arg("stream")
            .spawn()
            .map_err(|e| to_internal_status("START_REPLICA_BASEBACKUP_SPAWN", e))?;

        // restrict permissions to this user only on data dir, else pg_basebackup doesn't work
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| to_internal_status("START_REPLICA_PERMISSION_", e))?;


        let exit_status = pg_basebackup_process
            .wait().await
            .map_err(|e| to_internal_status("START_REPLICA_BASEBACKUP_EXECUTION", e))?;

        if !exit_status.success() {
            return Err(Status::internal(format!("pg_basebackup failed with status: {}", exit_status)));
        }

        // TODO: after a manager restart, state is empty even if a postgres is still running
        // on this data_dir (orphan from a crash). Detect via postmaster.pid (pid alive?)
        // and either adopt or refuse. Mostly prevented by graceful shutdown (see main.rs).

        let mut postgres_command = Command::new("postgres");
        let mut postgres_process = postgres_command
            .arg("-D")
            .arg(data_dir)
            .arg("-p")
            .arg(port.to_string())
            .spawn()
            .map_err(|e| to_internal_status("START_REPLICA_POSTGRES_SPAWN", e))?;

        let pid = match postgres_process.id() {
            Some(pid) => pid,
            None => return Err(Status::internal("postgres pid not available")),
        };

        // wait for postgres to be ready within a time period, else return error
        if let Err(e) = wait_until_pg_ready(&mut postgres_process, data_dir, "TEST_READINESS_FOR_REPLICA_PG")
            .await {
            let _ = postgres_process.kill().await;
            return Err(e);
        }

        *guard = Some(RunningInstance{port, child: postgres_process,
            data_dir: data_dir.to_string(), role: Replica});
        Ok(Response::new(StartReplicaResponse { pid }))
    }
    async fn status(&self, _request: Request<StatusRequest>)
                    -> Result<Response<StatusResponse>, Status> {
        let mut guard = self.running_instance.lock().await;
        let response = match  guard.as_mut() {
            Some(ri) => {
                match ri.child.try_wait() {
                    Ok(None) => StatusResponse {
                        state: State::Running as i32,
                        role: to_pb_role(ri.role) as i32,
                        port: ri.port,
                        pid: ri.child.id().unwrap_or(0),
                    },
                    Ok(Some(_)) => StatusResponse {
                        state: State::Exited as i32,
                        role: to_pb_role(ri.role) as i32,
                        port: ri.port,
                        pid: 0,
                    },
                    Err(e) => return Err(to_internal_status("STATUS_CHECK_CHILD_EXITED", e))
                }
            },
            None => StatusResponse {
                        state: State::NotStarted as i32,
                        role: PbRole::Unspecified as i32,
                        port: 0,
                        pid: 0
                }
        };

        Ok(Response::new(response))

    }
}

fn postgres_connection_string(user: &str, password: &str, host: &str, port: u32, database: &str) -> String {
    format!(
        "postgresql://{}:{}@{}:{}/{}",
        user, password, host, port, database
    )
}

// helper function, private to this file, no `pub`
fn to_internal_status(context: &str, e: std::io::Error) -> Status {
    eprintln!("{}: {}", context, e);
    Status::internal("Failure while trying to initialize data directory".to_string())
}

fn run_command(command: &str, args: &[&str], context: &str) -> Result<Child, Status> {
    Command::new(command)
        .args(args)
        .spawn()
        .map_err(|e| to_internal_status(context, e))
}

async fn wait_until_pg_ready(pg_process: &mut Child, data_dir: &str, context: &str) -> Result<(), Status> {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            return Err(Status::internal("Timeout while waiting for postgres to eb ready.".to_string()))
        }

        match pg_process.try_wait() {
            Ok(Some(status)) => {
                eprintln!("Postgres exited with status {}", status.to_string());
                return Err(Status::internal(format!("Postgres exited with status: {}", status.to_string())));
            },
            Ok(None) => {
                let pid_file = Path::new(data_dir).join("postmaster.pid");
                match tokio::fs::read_to_string(&pid_file).await {
                    Ok(contents) if matches!(contents.lines().last().map(str::trim), Some("ready") | Some("standby")) => {
                        return Ok(());
                    }
                    Ok(_) => {}, // file exists, but not ready yet
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}   // not written yet
                    Err(e) => return Err(to_internal_status("READ_POSTMASTER_PID", e)),
                }
            },
            Err(e) => return Err(to_internal_status(context, e)),
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn check_existing(slot: &mut Option<RunningInstance>, context: &str) -> Result<Found, Status> {
    match slot {
        Some(ri) => {
            match ri.child.try_wait() {
                Ok(Some(_)) => Ok(Found::Stale),    // child exited: stale record
                Ok(None)    => Ok(Found::Alive {role: ri.role, port: ri.port }),    // child still running
                Err(e)      => Err(to_internal_status(context, e)),
            }
        },
        None => Ok(Found::Nothing)
    }
}

fn to_pb_role(role: Role) -> PbRole {
    match role {
        Primary => PbRole::Primary,
        Replica => PbRole::Replica,
    }
}
