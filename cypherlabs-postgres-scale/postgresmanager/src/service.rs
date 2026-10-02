use std::path::{Path, PathBuf};
use tokio::process::Command;
use tonic::{async_trait, Request, Response, Status};
use tonic::codegen::Service;
use tonic::server::NamedService;
use crate::pb::postgres_manager_server::PostgresManager;
use crate::pb::{StartPrimaryRequest, StartPrimaryResponse};

pub(crate) struct PostgresManagerService;

impl PostgresManagerService {
    pub(crate) fn new() -> Self {
        PostgresManagerService
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


        Ok(Response::new(StartPrimaryResponse { pid }))
    }
}

// helper function, private to this file, no `pub`
fn to_internal_status(context: &str, e: std::io::Error) -> Status {
    eprintln!("{}: {}", context, e);
    Status::internal("Failure while trying to initialize data directory".to_string())
}

