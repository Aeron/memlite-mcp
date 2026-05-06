use anyhow::{Result, anyhow};

// Each entry migrates version N to N+1 as a list of SQL statements.
// Never edit existing entries; only append.
const MIGRATIONS: &[&[&str]] = &[
    // v0 -> v1: initial schema
    &[
        "create table if not exists memories (
            content    text    not null,
            summary    text,
            tags       text    not null default '[]',
            agent      text    not null default '',
            embedding  blob,
            created_at integer not null,
            updated_at integer not null
        )",
        "create index if not exists memories_agent on memories (agent)",
    ],
];

/// Applies all pending migrations, bumping `PRAGMA user_version` after each one.
pub async fn run(conn: &libsql::Connection) -> Result<()> {
    let version = user_version(conn).await?;
    for (i, stmts) in MIGRATIONS.iter().enumerate().skip(version) {
        for stmt in *stmts {
            conn.execute(stmt, ()).await?;
        }
        conn.execute(&format!("PRAGMA user_version = {}", i + 1), ())
            .await?;
    }
    Ok(())
}

async fn user_version(conn: &libsql::Connection) -> Result<usize> {
    let mut rows = conn.query("PRAGMA user_version", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("PRAGMA user_version returned no row"))?;
    let v: i64 = row.get(0)?;
    Ok(v as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;

    async fn mem_conn() -> libsql::Connection {
        Builder::new_local(":memory:")
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap()
    }

    #[tokio::test]
    async fn fresh_db_starts_at_version_zero() {
        let conn = mem_conn().await;
        assert_eq!(user_version(&conn).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn run_bumps_version_to_migration_count() {
        let conn = mem_conn().await;
        run(&conn).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), MIGRATIONS.len());
    }

    #[tokio::test]
    async fn run_is_idempotent() {
        let conn = mem_conn().await;
        run(&conn).await.unwrap();
        run(&conn).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), MIGRATIONS.len());
    }

    #[tokio::test]
    async fn run_creates_memories_table() {
        let conn = mem_conn().await;
        run(&conn).await.unwrap();
        // query() is used because SELECT returns rows; execute() rejects that
        let mut rows = conn
            .query("SELECT count(*) FROM memories", ())
            .await
            .unwrap();
        assert!(rows.next().await.unwrap().is_some());
    }
}
