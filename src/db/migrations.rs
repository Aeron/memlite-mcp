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
    // v8 -> v9: normalized tag table for indexed filtering (replaces json_each scan)
    &[
        "create table if not exists memory_tags (
            memory_id integer not null,
            tag       text    not null,
            primary key (memory_id, tag)
        )",
        "create index if not exists memory_tags_tag on memory_tags (tag)",
        // Populating from existing JSON column; on conflict = already there, skip
        "insert or ignore into memory_tags (memory_id, tag)
            select m.rowid, jt.value
            from memories m, json_each(m.tags) jt
            where trim(jt.value) != ''",
    ],
    // v9 -> v10: normalized relation table for indexed graph queries
    &[
        "create table if not exists memory_relations (
            from_id integer not null,
            to_id   integer not null,
            kind    text    not null default '',
            primary key (from_id, to_id, kind)
        )",
        "create index if not exists memory_relations_to on memory_relations (to_id)",
        // Backfilling from existing JSON relations column; keys are stringified integer ids
        "insert or ignore into memory_relations (from_id, to_id, kind)
            select m.rowid, cast(jt.key as integer), jt.value
            from memories m, json_each(m.relations) jt
            where trim(jt.key) != '' and cast(jt.key as integer) > 0",
    ],
    // v10 -> v11: external reference pointers (file paths, symbol names, URLs, etc.)
    &["alter table memories add column refs text not null default '[]'"],
    // v11 -> v12: removes per-row agent column (isolation handled at DB file level)
    &[
        "drop index if exists memories_agent",
        "alter table memories drop column agent",
    ],
    // v12 -> v13: merges status/severity columns into the free-form attrs JSON bag;
    // filtering still works via json_extract(attrs, '$.status') etc.
    &[
        "alter table memories add column attrs text not null default '{}'",
        "update memories set attrs = (
            case
                when status is not null and severity is not null
                    then json_object('status', status, 'severity', severity)
                when status is not null
                    then json_object('status', status)
                when severity is not null
                    then json_object('severity', severity)
                else '{}'
            end
        ) where status is not null or severity is not null",
        "alter table memories drop column status",
        "alter table memories drop column severity",
    ],
];

const SCHEMA_VERSION: usize = MIGRATIONS.len() + 1;

/// Applies pending schema changes and validates the configured embedding model.
///
/// Re-reading the version after acquiring each write transaction makes startup
/// safe when multiple server processes open a new project database concurrently.
pub async fn run(
    conn: &libsql::Connection,
    model_name: &str,
    embedding_dimensions: usize,
) -> Result<()> {
    loop {
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        // Applying one version per transaction so concurrent startup can resume safely
        let version = transaction_user_version(&tx).await?;
        let done = if version > SCHEMA_VERSION {
            tracing::warn!(
                db_version = version,
                code_version = SCHEMA_VERSION,
                "database schema is newer than this binary; some fields may be unrecognised"
            );
            check_embedding_model(&tx, model_name).await?;
            true
        } else if version < MIGRATIONS.len() {
            for stmt in MIGRATIONS[version] {
                tx.execute(stmt, ()).await?;
            }
            tx.execute(&format!("pragma user_version = {}", version + 1), ())
                .await?;
            false
        } else {
            check_embedding_model(&tx, model_name).await?;
            if version < SCHEMA_VERSION {
                migrate_vector_index(&tx, embedding_dimensions).await?;
                tx.execute(&format!("pragma user_version = {SCHEMA_VERSION}"), ())
                    .await?;
            }
            true
        };

        tx.commit().await?;
        if done {
            return Ok(());
        }
    }
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

async fn migrate_vector_index(tx: &libsql::Transaction, embedding_dimensions: usize) -> Result<()> {
    if embedding_dimensions == 0 {
        return Err(anyhow!("embedding dimensions must be greater than zero"));
    }

    // Removing invalid normalized values before rebuilding canonical JSON columns
    tx.execute("delete from memory_tags where trim(tag) = ''", ())
        .await?;
    tx.execute(
        "delete from memory_relations
         where not exists (select 1 from memories where rowid = memory_relations.to_id)",
        (),
    )
    .await?;
    tx.execute(
        "update memories set tags = coalesce(
            (select json_group_array(tag) from memory_tags where memory_id = memories.rowid),
            '[]'
        )",
        (),
    )
    .await?;
    tx.execute(
        "update memories set relations = coalesce(
            (select json_group_object(to_id, kind)
             from memory_relations where from_id = memories.rowid),
            '{}'
        )",
        (),
    )
    .await?;

    // Rebuilding the table because SQLite cannot alter a column's declared type
    tx.execute(
        &format!(
            "create table memories_v14 (
                content      text not null,
                summary      text,
                tags         text not null default '[]',
                embedding    f32_blob({embedding_dimensions}),
                created_at   integer not null,
                updated_at   integer not null,
                context      text,
                confidence   real not null default 1.0,
                decay_rate   real not null default 0.0,
                last_seen_at integer not null default 0,
                type         text,
                relations    text not null default '{{}}',
                refs         text not null default '[]',
                attrs        text not null default '{{}}'
            )"
        ),
        (),
    )
    .await?;
    tx.execute(
        "insert into memories_v14 (
            rowid, content, summary, tags, embedding, created_at, updated_at, context,
            confidence, decay_rate, last_seen_at, type, relations, refs, attrs
        )
        select
            rowid, content, summary, tags, embedding, created_at, updated_at, context,
            confidence, decay_rate, last_seen_at, type, relations, refs, attrs
        from memories",
        (),
    )
    .await?;
    tx.execute("drop table memories", ()).await?;
    tx.execute("alter table memories_v14 rename to memories", ())
        .await?;
    tx.execute(
        "create index memories_embedding_idx on memories (
            libsql_vector_idx(embedding, 'metric=cosine')
        )",
        (),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
async fn user_version(conn: &libsql::Connection) -> Result<usize> {
    let mut rows = conn.query("pragma user_version", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("pragma user_version returned no row"))?;
    let v: i64 = row.get(0)?;
    Ok(v as usize)
}

async fn transaction_user_version(tx: &libsql::Transaction) -> Result<usize> {
    let mut rows = tx.query("pragma user_version", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("pragma user_version returned no row"))?;
    let value: i64 = row.get(0)?;
    Ok(value as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;
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
    async fn run_creates_memories_table() {
        let conn = mem_conn().await;
        run(&conn, "test-model", 2).await.unwrap();
        // Using query() because select returns rows while execute() rejects them
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
    async fn vector_migration_indexes_existing_rows_and_repairs_drift() {
        let conn = mem_conn().await;
        for (version, statements) in MIGRATIONS.iter().enumerate() {
            let tx = conn
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await
                .unwrap();
            for statement in *statements {
                tx.execute(statement, ()).await.unwrap();
            }
            tx.execute(&format!("pragma user_version = {}", version + 1), ())
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }

        let embedding = crate::db::helpers::floats_to_blob(&[1.0, 0.0]);
        conn.execute(
            "insert into memories (
                content, tags, embedding, created_at, updated_at, type
             ) values ('existing', '[\"rust\",\"\",\"rust\"]', ?, 1, 1, 'project')",
            libsql::params![embedding.clone()],
        )
        .await
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "insert or ignore into memory_tags (memory_id, tag)
             values (?, 'rust'), (?, '')",
            libsql::params![id, id],
        )
        .await
        .unwrap();
        conn.execute(
            "insert into memories (
                content, tags, created_at, updated_at, type, relations
             ) values ('stale relation', '[]', 1, 1, 'project', '{\"999\":\"related_to\"}')",
            (),
        )
        .await
        .unwrap();
        let stale_source_id = conn.last_insert_rowid();
        conn.execute(
            "insert into memory_relations (from_id, to_id, kind)
             values (?, 999, 'related_to')",
            libsql::params![stale_source_id],
        )
        .await
        .unwrap();

        run(&conn, "test-model", 2).await.unwrap();

        let mut rows = conn
            .query(
                "select id from vector_top_k('memories_embedding_idx', ?, 10)",
                libsql::params![embedding],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            id
        );
        let mut rows = conn
            .query(
                "select tags from memories where rowid = ?",
                libsql::params![id],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            r#"["rust"]"#
        );
        let mut rows = conn
            .query(
                "select relations from memories where rowid = ?",
                libsql::params![stale_source_id],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "{}"
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
