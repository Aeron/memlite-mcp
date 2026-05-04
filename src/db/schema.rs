use crate::error::Result;

const CREATE_MEMORIES: &str = "
    CREATE TABLE IF NOT EXISTS memories (
        content    TEXT    NOT NULL,
        summary    TEXT,
        tags       TEXT    NOT NULL DEFAULT '[]',
        agent_id   TEXT    NOT NULL DEFAULT '',
        embedding  BLOB,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    )
";

const CREATE_AGENT_IDX: &str = "
    CREATE INDEX IF NOT EXISTS memories_agent_id ON memories (agent_id)
";

/// Creates the memories table and supporting indexes on the given connection.
pub async fn init(conn: &libsql::Connection) -> Result<()> {
    conn.execute(CREATE_MEMORIES, ()).await?;
    conn.execute(CREATE_AGENT_IDX, ()).await?;
    Ok(())
}
