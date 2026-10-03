use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tonic::{async_trait, Request, Response, Status};
use crate::pb::postgres_manager_server::PostgresManager;
use crate::pb::{CreateReplicationSlotRequest, CreateReplicationSlotResponse, StartPrimaryRequest, StartPrimaryResponse};

pub(crate) struct PostgresManagerService {
    postgres_port: Arc<Mutex<Option<u32>>>
}

impl PostgresManagerService {
    pub(crate) fn new() -> Self {
        let postgres_port = Arc::new(Mutex::new(None));
        PostgresManagerService {
            postgres_port
        }
    }
}

#[async_trait]
impl PostgresManager for PostgresManagerService {

    async fn start_primary(&self, request: Request<StartPrimaryRequest>)
        -> Result<Response<StartPrimaryResponse>, Status> {
        let start_primary_request = request.get_ref();
        let data_dir = &start_primary_request.data_dir;
        let port = start_primary_request.port;

        let is_data_dir_initialized = Path::exists(PathBuf::from(data_dir).join("PG_VERSION").as_path());
        if !is_data_dir_initialized {
            let mut initdb_command = Command::new("initdb");
            let mut initdb_process = initdb_command
                .arg("-D")
                .arg(data_dir)
                .spawn()
                .map_err(|e| to_internal_status("START_PRIMARY_INITDB_SPAWN", e))?;
            let pid = match initdb_process.id() {
                Some(pid) => pid,
                None => return Err(Status::internal("PID not set")),
            };

            let exitstatus = initdb_process.wait().await
                .map_err(|e| to_internal_status("START_PRIMARY_INITDB_EXECUTION", e))?;
            if !exitstatus.success() {
                return Err(Status::internal(format!("DB initialization failed with error {}",
                                                    exitstatus.to_string())));
            }
        }

        // TODO(v0 gap): if a postgres instance is already running against this data_dir
        // (detectable via postmaster.pid's pid still being alive), StartPrimary currently
        // has no check for this and will attempt to spawn a second postgres, which will
        // fail due to port/lock conflicts — but since postgres's spawn() succeeding doesn't
        // mean it stayed running, the current code would still report a false-positive
        // success (returning a pid for a process that immediately exited).
        // Decision deferred: should a call against an already-running instance be (a) an
        // error (FAILED_PRECONDITION/ALREADY_EXISTS), or (b) idempotent success (return the
        // existing pid without spawning)? Revisit when Operator's real retry semantics are known.

        let mut postgres_command = Command::new("postgres");
        let postgres_process = postgres_command
            .arg("-D")
            .arg(data_dir)
            .arg("-p")
            .arg(port.to_string())
            .spawn()
            .map_err(|e| to_internal_status("START_PRIMARY_POSTGRES_SPAWN", e))?;

        let pid = match postgres_process.id() {
            Some(pid) => pid,
            None => return Err(Status::internal("postgres pid not available")),
        };

        *self.postgres_port.lock().await = Some(port);
        Ok(Response::new(StartPrimaryResponse { pid }))
    }

    async fn create_replication_slot(&self, request: Request<CreateReplicationSlotRequest>)
        -> Result<Response<CreateReplicationSlotResponse>, Status> {
        let create_replication_slot_request = request.get_ref();
        let guard = self.postgres_port.lock().await;
        let port = match *guard {
            Some(port) => port,
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

