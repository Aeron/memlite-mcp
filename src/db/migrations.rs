use anyhow::{Result, anyhow};

/// SQL statements grouped by source schema version
const MIGRATIONS: &[&[&str]] = &[&[
    "create table memories (
        id           integer primary key autoincrement,
        content      text not null,
        summary      text not null,
        tags         text not null default '[]',
        embedding    f32_blob({embedding_dimensions}),
        created_at   integer not null,
        updated_at   integer not null,
        context      text,
        confidence   real not null default 1.0,
        decay_rate   real not null default 0.0,
        last_seen_at integer not null default 0,
        type         text,
        relations    text not null default '{}',
        refs         text not null default '[]',
        attrs        text not null default '{}'
    )",
    "create index memories_embedding_idx on memories (
        libsql_vector_idx(embedding, 'metric=cosine')
    )",
    "create table meta (
        key   text primary key,
        value text not null
    )",
    "create table memory_tags (
        memory_id integer not null,
        tag       text not null,
        primary key (memory_id, tag)
    )",
    "create index memory_tags_tag on memory_tags (tag)",
    "create table memory_relations (
        from_id integer not null,
        to_id   integer not null,
        kind    text not null default '',
        primary key (from_id, to_id, kind)
    )",
    "create index memory_relations_to on memory_relations (to_id)",
]];

const SCHEMA_VERSION: usize = MIGRATIONS.len();

/// Applies the initial schema and validates the configured embedding model.
///
/// Reading the version after acquiring the write transaction makes startup safe
/// when multiple server processes open a new project database concurrently.
pub async fn run(
    conn: &libsql::Connection,
    model_name: &str,
    embedding_dimensions: usize,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    let version = check_version(&tx).await?;

    if version < SCHEMA_VERSION {
        if embedding_dimensions == 0 {
            return Err(anyhow!("embedding dimensions must be greater than zero"));
        }
        let embedding_dimensions = embedding_dimensions.to_string();

        // Applying pending migrations in current write transaction
        for migration in MIGRATIONS.iter().skip(version) {
            for statement in *migration {
                tx.execute(
                    &statement.replace("{embedding_dimensions}", &embedding_dimensions),
                    (),
                )
                .await?;
            }
        }
        tx.execute(&format!("pragma user_version = {SCHEMA_VERSION}"), ())
            .await?;
    }

    // Validating compressed schema and embedding model
    check_compressed_schema(&tx, version).await?;
    check_embedding_model(&tx, model_name).await?;
    tx.commit().await?;
    Ok(())
}

async fn check_compressed_schema(tx: &libsql::Transaction, version: usize) -> Result<()> {
    let mut rows = tx.query("pragma table_info(memories)", ()).await?;
    while let Some(row) = rows.next().await? {
        let column: String = row.get(1)?;
        if column == "summary" {
            let not_null: i64 = row.get(3)?;
            return if not_null == 1 {
                Ok(())
            } else {
                Err(anyhow!(
                    "database schema version {version} is incompatible with compressed v1: summary must be NOT NULL"
                ))
            };
        }
    }
    Err(anyhow!(
        "database schema version {version} is incompatible with compressed v1: summary column is missing"
    ))
}

async fn check_embedding_model(tx: &libsql::Transaction, model_name: &str) -> Result<()> {
    tx.execute(
        "insert or ignore into meta (key, value) values ('embedding_model', ?)",
        libsql::params![model_name],
    )
    .await?;
    let mut rows = tx
        .query("select value from meta where key = 'embedding_model'", ())
        .await?;
    let stored: String = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("embedding model metadata was not recorded"))?
        .get(0)?;
    if stored != model_name {
        return Err(anyhow!(
            "embedding model mismatch: DB was created with '{}' but '{}' was requested; use the original model or delete the database to start fresh",
            stored,
            model_name,
        ));
    }
    Ok(())
}

#[cfg(test)]
async fn user_version(conn: &libsql::Connection) -> Result<usize> {
    let mut rows = conn.query("pragma user_version", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("pragma user_version returned no row"))?;
    let version: i64 = row.get(0)?;
    Ok(version as usize)
}

/// Rejects unsupported newer schemas without changing database settings or data.
pub(super) async fn check_version(conn: &libsql::Connection) -> Result<usize> {
    let mut rows = conn.query("pragma user_version", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("pragma user_version returned no row"))?;
    let version = row.get::<i64>(0)? as usize;
    if version > SCHEMA_VERSION {
        return Err(anyhow!(
            "database schema version {version} is newer than supported version {SCHEMA_VERSION}; upgrade memlite before opening this database"
        ));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use libsql::{Builder, OpenFlags};
    use tempfile::NamedTempFile;

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
    async fn run_bumps_version_to_schema_version() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn run_is_idempotent() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        run(&conn, "test-model", 2).await.unwrap();
        assert_eq!(user_version(&conn).await.unwrap(), SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn open_rejects_newer_schema_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = NamedTempFile::new_in(dir.path()).unwrap();
        let conn = Builder::new_local(file.path())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        run(&conn, "test-model", 2).await.unwrap();
        let mut rows = conn
            .query("pragma journal_mode = delete", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "delete"
        );
        drop(rows);

        // Preparing future-schema data with missing model metadata
        conn.execute(
            "insert into memories (content, summary, created_at, updated_at)
             values ('future content', 'future summary', 1, 1)",
            (),
        )
        .await
        .unwrap();
        conn.execute("delete from meta", ()).await.unwrap();
        let version = SCHEMA_VERSION + 1;
        conn.execute(&format!("pragma user_version = {version}"), ())
            .await
            .unwrap();

        drop(conn);
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let before = std::fs::read(file.path()).unwrap();

        let error = crate::db::MemoryStore::open(file.path(), "test-model", 2)
            .await
            .err()
            .expect("newer schema should fail to open");
        assert_eq!(
            error.to_string(),
            format!(
                "database schema version {version} is newer than supported version {SCHEMA_VERSION}; upgrade memlite before opening this database"
            )
        );

        // Checking rejection leaves journal mode, permissions and database bytes unchanged
        assert_eq!(
            std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o7777,
            0o644
        );
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = file.path().as_os_str().to_os_string();
            sidecar.push(suffix);
            assert!(!std::path::Path::new(&sidecar).exists());
        }
        let conn = Builder::new_local(file.path())
            .flags(OpenFlags::SQLITE_OPEN_READ_ONLY)
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let mut rows = conn.query("pragma journal_mode", ()).await.unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "delete"
        );
        drop(rows);
        assert_eq!(std::fs::read(file.path()).unwrap(), before);

        // Checking rejection leaves memory data, metadata and version unchanged
        assert_eq!(user_version(&conn).await.unwrap(), version);
        let mut rows = conn
            .query(
                "select content, summary, (select count(*) from meta) from memories",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "future content");
        assert_eq!(row.get::<String>(1).unwrap(), "future summary");
        assert_eq!(row.get::<i64>(2).unwrap(), 0);
        assert!(rows.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn run_rejects_newer_schema_inside_transaction() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        conn.execute("delete from meta", ()).await.unwrap();
        let version = SCHEMA_VERSION + 1;
        conn.execute(&format!("pragma user_version = {version}"), ())
            .await
            .unwrap();

        let error = run(&conn, "test-model", 2).await.unwrap_err();
        assert!(error.to_string().contains("newer than supported"));
        assert_eq!(user_version(&conn).await.unwrap(), version);
        let mut rows = conn.query("select count(*) from meta", ()).await.unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn run_creates_memories_table() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        let mut rows = conn
            .query("select count(*) from memories", ())
            .await
            .unwrap();
        assert!(rows.next().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn run_creates_meta_table() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        let mut rows = conn.query("select count(*) from meta", ()).await.unwrap();
        assert!(rows.next().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn run_rejects_uncompressed_legacy_schema() {
        let file = NamedTempFile::new().unwrap();
        let conn = Builder::new_local(file.path())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        conn.execute(
            "create table memories (
                content text not null,
                summary text,
                created_at integer not null,
                updated_at integer not null
            )",
            (),
        )
        .await
        .unwrap();
        conn.execute("pragma user_version = 1", ()).await.unwrap();

        let error = run(&conn, "test-model", 2).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "database schema version 1 is incompatible with compressed v1: summary must be NOT NULL"
        );
    }

    #[tokio::test]
    async fn run_requires_summary() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();

        let error = conn
            .execute(
                "insert into memories (content, created_at, updated_at) values ('content', 1, 1)",
                (),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("NOT NULL constraint failed"));
    }

    #[tokio::test]
    async fn run_creates_dimensioned_vector_index() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        let mut rows = conn.query("pragma table_info(memories)", ()).await.unwrap();
        let mut embedding_type = None;
        while let Some(row) = rows.next().await.unwrap() {
            if row.get::<String>(1).unwrap() == "embedding" {
                embedding_type = Some(row.get::<String>(2).unwrap());
            }
        }
        assert_eq!(embedding_type.as_deref(), Some("f32_blob(2)"));

        let mut rows = conn
            .query(
                "select count(*) from sqlite_master
                 where type = 'index' and name = 'memories_embedding_idx'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn concurrent_open_is_migration_and_model_safe() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_owned();
        let first = crate::db::MemoryStore::open(&path, "test-model", 2);
        let second = crate::db::MemoryStore::open(&path, "test-model", 2);
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
    }
}
