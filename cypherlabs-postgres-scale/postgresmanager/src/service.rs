use std::path::{Path, PathBuf};
use tokio::process::Command;
use tonic::{Request, Response, Status};
use crate::pb::postgres_manager_server::PostgresManager;
use crate::pb::{StartPrimaryRequest, StartPrimaryResponse};

struct PostgresManagerService;

impl PostgresManager for PostgresManagerService {
    async fn start_primary(&self, request: Request<StartPrimaryRequest>)
        -> Result<Response<StartPrimaryResponse>, Status> {
        let start_primary_request = request.get_ref();
        let data_dir = &start_primary_request.data_dir;
        let port = start_primary_request.port;

        let is_data_dir_initialized = Path::exists(PathBuf::from(data_dir).join("PG_VERSION").as_path());

        if is_data_dir_initialized {

        } else {
            let initdb_command = &mut Command::new("inintdb");
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
            if exitstatus.success() {
                return Ok(Response::new(crate::pb::StartPrimaryResponse(pid)))
            }
        }


        Ok(Response::new(StartPrimaryResponse { pid: 1 }))
    }
}

// helper function, private to this file, no `pub`
fn to_internal_status(context: &str, e: std::io::Error) -> Status {
    eprintln!("{}: {}", context, e);
    Status::internal("Failure while trying to initialize data directory".to_string())
}

