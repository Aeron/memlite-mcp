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
    // v1 -> v2: adds optional context column for storing background rationale
    &["alter table memories add column context text"],
    // v2 -> v3: adds confidence decay fields
    &[
        "alter table memories add column confidence real not null default 1.0",
        "alter table memories add column decay_rate real not null default 0.0",
        "alter table memories add column last_seen_at integer not null default 0",
    ],
    // v3 -> v4: adds structured memory type field
    &["alter table memories add column type text"],
    // v4 -> v5: adds lifecycle status field
    &["alter table memories add column status text"],
    // v5 -> v6: adds severity field
    &["alter table memories add column severity text"],
    // v6 -> v7: adds directed outgoing relation links
    &["alter table memories add column relations text not null default '{}'"],
    // v7 -> v8: adds key-value metadata table for DB-level settings (e.g. embedding model)
    &["create table if not exists meta (key text primary key, value text not null)"],
];

/// Applies all pending migrations, each wrapped in an explicit transaction.
///
/// Wrapping each step atomically means a crash between statements leaves the
/// database at the previous version; on restart the same step is retried
/// cleanly from a known-good state rather than encountering half-applied DDL.
pub async fn run(conn: &libsql::Connection) -> Result<()> {
    let version = user_version(conn).await?;
    let total = MIGRATIONS.len();
    if version > total {
        tracing::warn!(
            db_version = version,
            code_version = total,
            "database schema is newer than this binary; some fields may be unrecognised"
        );
    }
    for (i, stmts) in MIGRATIONS.iter().enumerate().skip(version) {
        conn.execute("begin immediate", ()).await?;
        let result: Result<()> = async {
            for stmt in *stmts {
                conn.execute(stmt, ()).await?;
            }
            conn.execute(&format!("pragma user_version = {}", i + 1), ())
                .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => conn.execute("commit", ()).await?,
            Err(e) => {
                let _ = conn.execute("rollback", ()).await;
                return Err(e);
            }
        };
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

    #[tokio::test]
    async fn run_creates_meta_table() {
        let conn = mem_conn().await;
        run(&conn).await.unwrap();
        let mut rows = conn.query("SELECT count(*) FROM meta", ()).await.unwrap();
        assert!(rows.next().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn run_version_matches_migration_count() {
        let conn = mem_conn().await;
        run(&conn).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), MIGRATIONS.len());
    }

    #[tokio::test]
    async fn failed_migration_leaves_version_unchanged() {
        let conn = mem_conn().await;
        // Running partial migrations first (to v1) by hand
        conn.execute("begin immediate", ()).await.unwrap();
        for stmt in MIGRATIONS[0] {
            conn.execute(stmt, ()).await.unwrap();
        }
        conn.execute("pragma user_version = 1", ()).await.unwrap();
        conn.execute("commit", ()).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), 1);

        // Simulating a bad migration (invalid SQL) at the next step
        conn.execute("begin immediate", ()).await.unwrap();
        let bad_result = conn.execute("this is not valid sql", ()).await;
        assert!(bad_result.is_err());
        conn.execute("rollback", ()).await.unwrap();

        // Version must still be 1; the bad step was rolled back
        assert_eq!(user_version(&conn).await.unwrap(), 1);
    }
}
