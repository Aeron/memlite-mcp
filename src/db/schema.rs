use anyhow::Result;

const CREATE_MEMORIES: &str = "
    create table if not exists memories (
        content    text    not null,
        summary    text,
        tags       text    not null default '[]',
        agent      text    not null default '',
        embedding  blob,
        created_at integer not null,
        updated_at integer not null
    )
";

const CREATE_AGENT_IDX: &str = "
    create index if not exists memories_agent on memories (agent)
";

/// Creates the memories table and supporting indexes on the given connection.
pub async fn init(conn: &libsql::Connection) -> Result<()> {
    conn.execute(CREATE_MEMORIES, ()).await?;
    conn.execute(CREATE_AGENT_IDX, ()).await?;
    Ok(())
}
