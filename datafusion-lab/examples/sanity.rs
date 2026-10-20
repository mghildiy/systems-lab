use datafusion::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    datafusion_lab::basics::sanity_run().await
}