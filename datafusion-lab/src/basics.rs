use datafusion::error::Result;
use datafusion::prelude::*;

pub async fn sanity_run() -> Result<()> {
    let ctx = SessionContext::new();
    ctx.sql("SELECT 1 + 1 AS answer").await?.show().await?;
    Ok(())
}

pub async fn csv_run() -> Result<()> {
    let ctx = SessionContext::new();
    ctx.register_csv("people", "data/people.csv", CsvReadOptions::new()).await?;

    ctx.sql("SELECT city, COUNT(*) AS n, AVG(age) AS avg_age FROM people GROUP BY city")
        .await?
        .show()
        .await?;

    ctx.sql("EXPLAIN SELECT city, COUNT(*) FROM people WHERE age > 26 GROUP BY city")
        .await?
        .show()
        .await?;

    Ok(())
}