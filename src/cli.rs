use std::{io::Write, str::FromStr};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

use crate::{
    config::Config,
    db::{
        BatchInsertItem, FilterOptions, MemoryAttrOutcome, MemoryAttrPhase, MemoryAttrPriority,
        MemoryAttrSeverity, MemoryAttrs, MemoryStatus, MemoryStore, MemoryType, TraverseDirection,
        UpdateOptions,
    },
    embedding::EmbeddingService,
    output::{
        compact_list_json, hubs_json, list_json, neighbors_json, path_json,
        render_compact_list_results, render_list_results, render_search_results, render_toon,
        search_json, stats_json, traverse_json,
    },
};

/// Parses top-level server and memory management commands.
#[derive(Parser)]
#[command(name = "memlite", version, about)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) config: Config,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Selects server mode or direct memory management.
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Runs the MCP server over stdio.
    Serve,
    /// Runs one memory tool directly against the project database.
    Tool(Box<ToolArgs>),
}

/// Holds direct tool output and command selection.
#[derive(Args)]
pub(crate) struct ToolArgs {
    /// Output format for direct tool commands.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    command: ToolCommand,
}

/// Selects a direct memory operation.
#[derive(Subcommand)]
enum ToolCommand {
    /// Stores one memory.
    Store(StoreArgs),
    /// Searches memories by semantic similarity.
    Search(SearchArgs),
    /// Lists memories newest first.
    List(ListArgs),
    /// Lists tags and occurrence counts.
    Tags,
    /// Deletes one memory.
    Delete(DeleteArgs),
    /// Updates one memory.
    Update(Box<UpdateArgs>),
    /// Resets confidence decay clocks.
    Revalidate(RevalidateArgs),
    /// Lists immediate incoming and outgoing neighbors.
    Neighbors(NeighborsArgs),
    /// Traverses the relation graph breadth first.
    Traverse(TraverseArgs),
    /// Finds the shortest directed path between memories.
    Path(PathArgs),
    /// Lists the most connected memories.
    Hubs(HubsArgs),
    /// Shows graph-level statistics.
    Stats,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Args)]
struct StoreArgs {
    /// Full text to embed and store.
    content: String,

    /// Structured memory category.
    #[arg(long)]
    memory_type: MemoryType,

    /// One-sentence description for list views.
    #[arg(long)]
    summary: String,

    /// Grouping label; repeat for multiple tags.
    #[arg(long = "tag")]
    tags: Vec<String>,

    /// Background rationale.
    #[arg(long)]
    context: Option<String>,

    /// Initial confidence score in the range 0 to 1.
    #[arg(long)]
    confidence: Option<f64>,

    /// Per-day confidence decay rate in the range 0 to 1.
    #[arg(long)]
    decay_rate: Option<f64>,

    #[command(flatten)]
    attrs: AttrArgs,

    /// Outgoing relation in ID:KIND form; repeat for multiple relations.
    #[arg(long = "relation", value_name = "ID:KIND")]
    relations: Vec<Relation>,

    /// External reference; repeat for multiple references.
    #[arg(long = "ref")]
    refs: Vec<String>,
}

#[derive(Args)]
struct SearchArgs {
    /// Natural-language query.
    query: String,

    /// Maximum number of results.
    #[arg(long)]
    limit: Option<u32>,

    /// Includes full context and content.
    #[arg(long, action = ArgAction::Set)]
    include_content: Option<bool>,

    #[command(flatten)]
    filters: FilterArgs,
}

#[derive(Args)]
struct ListArgs {
    /// Exact memory identifiers; missing identifiers are omitted.
    ids: Vec<i64>,

    /// Maximum number of results.
    #[arg(long)]
    limit: Option<u32>,

    /// Zero-based pagination offset.
    #[arg(long)]
    offset: Option<u32>,

    /// Includes full context and content.
    #[arg(long, action = ArgAction::Set)]
    include_content: Option<bool>,

    #[command(flatten)]
    filters: FilterArgs,
}

#[derive(Args)]
struct DeleteArgs {
    /// Numeric memory identifier.
    id: i64,
}

#[derive(Args)]
struct UpdateArgs {
    /// Numeric memory identifier.
    id: i64,

    /// Replacement content; triggers re-embedding.
    #[arg(long)]
    content: Option<String>,

    /// Replacement summary; blank values are rejected.
    #[arg(long)]
    summary: Option<String>,

    /// Replacement tag; repeat for multiple tags.
    #[arg(long = "tag")]
    tags: Vec<String>,

    /// Clears all tags.
    #[arg(long)]
    clear_tags: bool,

    /// Replacement context; pass an empty string to clear.
    #[arg(long)]
    context: Option<String>,

    /// Replacement confidence score in the range 0 to 1.
    #[arg(long)]
    confidence: Option<f64>,

    /// Replacement per-day decay rate in the range 0 to 1.
    #[arg(long)]
    decay_rate: Option<f64>,

    /// Replacement memory category.
    #[arg(long)]
    memory_type: Option<MemoryType>,

    #[command(flatten)]
    attrs: AttrArgs,

    /// Clears all typed attributes.
    #[arg(long)]
    clear_attrs: bool,

    /// Replacement relation in ID:KIND form; repeat for multiple relations.
    #[arg(long = "relation", value_name = "ID:KIND")]
    relations: Vec<Relation>,

    /// Clears all outgoing relations.
    #[arg(long)]
    clear_relations: bool,

    /// Replacement external reference; repeat for multiple references.
    #[arg(long = "ref")]
    refs: Vec<String>,

    /// Clears all external references.
    #[arg(long)]
    clear_refs: bool,
}

#[derive(Args)]
struct RevalidateArgs {
    /// Numeric memory identifiers.
    #[arg(required = true, num_args = 1..=100)]
    ids: Vec<i64>,
}

#[derive(Args)]
struct NeighborsArgs {
    /// Numeric memory identifier.
    id: i64,

    /// Exact relation kind filter.
    #[arg(long)]
    kind: Option<String>,
}

#[derive(Args)]
struct TraverseArgs {
    /// Numeric starting memory identifier.
    start_id: i64,

    /// Traversal depth from 1 through 5.
    #[arg(long)]
    depth: Option<u32>,

    /// Exact relation kind filter.
    #[arg(long)]
    kind: Option<String>,

    /// Edge direction to follow.
    #[arg(long, value_enum)]
    direction: Option<TraverseDirection>,
}

#[derive(Args)]
struct PathArgs {
    /// Numeric starting memory identifier.
    from_id: i64,

    /// Numeric destination memory identifier.
    to_id: i64,

    /// Maximum hops from 1 through 10.
    #[arg(long)]
    max_hops: Option<u32>,
}

#[derive(Args)]
struct HubsArgs {
    /// Maximum hub count from 1 through 50.
    #[arg(long)]
    top_n: Option<u32>,
}

#[derive(Args, Default)]
struct FilterArgs {
    /// Required tag; repeat for AND filtering.
    #[arg(long = "tag")]
    tags: Vec<String>,

    /// Required memory category.
    #[arg(long)]
    memory_type: Option<MemoryType>,

    /// Required lifecycle status.
    #[arg(long)]
    status: Option<MemoryStatus>,

    /// Required severity.
    #[arg(long)]
    severity: Option<MemoryAttrSeverity>,

    /// Required priority.
    #[arg(long)]
    priority: Option<MemoryAttrPriority>,

    /// Required decision outcome.
    #[arg(long)]
    outcome: Option<MemoryAttrOutcome>,

    /// Required plan phase.
    #[arg(long)]
    phase: Option<MemoryAttrPhase>,

    /// Required substring in an external reference.
    #[arg(long)]
    refs_contain: Option<String>,
}

#[derive(Args, Default)]
struct AttrArgs {
    /// Lifecycle status.
    #[arg(long)]
    status: Option<MemoryStatus>,

    /// Severity level.
    #[arg(long)]
    severity: Option<MemoryAttrSeverity>,

    /// Task priority.
    #[arg(long)]
    priority: Option<MemoryAttrPriority>,

    /// Decision outcome.
    #[arg(long)]
    outcome: Option<MemoryAttrOutcome>,

    /// Decision reversibility.
    #[arg(long, action = ArgAction::Set)]
    reversible: Option<bool>,

    /// Plan phase.
    #[arg(long)]
    phase: Option<MemoryAttrPhase>,
}

#[derive(Clone)]
struct Relation {
    id: i64,
    kind: String,
}

impl FromStr for Relation {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, kind) = value
            .split_once(':')
            .ok_or_else(|| "expected ID:KIND".to_owned())?;
        let id = id
            .parse::<i64>()
            .map_err(|_| "relation identifier must be numeric".to_owned())?;
        if id <= 0 {
            return Err("relation identifier must be positive".to_owned());
        }
        if kind.trim().is_empty() {
            return Err("relation kind must not be blank".to_owned());
        }
        Ok(Relation {
            id,
            kind: kind.to_owned(),
        })
    }
}

impl FilterArgs {
    fn options<'a>(&'a self, ids: Option<&'a [i64]>) -> FilterOptions<'a> {
        FilterOptions {
            ids,
            tags: (!self.tags.is_empty()).then_some(self.tags.as_slice()),
            memory_type: self.memory_type,
            status: self.status,
            severity: self.severity,
            priority: self.priority,
            outcome: self.outcome,
            phase: self.phase,
            refs_contain: self.refs_contain.as_deref(),
        }
    }
}

impl AttrArgs {
    fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.severity.is_none()
            && self.priority.is_none()
            && self.outcome.is_none()
            && self.reversible.is_none()
            && self.phase.is_none()
    }

    fn value(&self) -> MemoryAttrs {
        MemoryAttrs {
            status: self.status,
            severity: self.severity,
            priority: self.priority,
            outcome: self.outcome,
            reversible: self.reversible,
            phase: self.phase,
        }
    }
}

/// Runs one direct memory tool and writes its result to stdout.
pub(crate) async fn run(config: &Config, args: ToolArgs) -> Result<()> {
    let format = args.format;
    match args.command {
        ToolCommand::Store(args) => store(config, format, args).await,
        ToolCommand::Search(args) => search(config, format, args).await,
        ToolCommand::List(args) => list(config, format, args).await,
        ToolCommand::Tags => tags(config, format).await,
        ToolCommand::Delete(args) => delete(config, format, args).await,
        ToolCommand::Update(args) => update(config, format, *args).await,
        ToolCommand::Revalidate(args) => revalidate(config, format, args).await,
        ToolCommand::Neighbors(args) => neighbors(config, format, args).await,
        ToolCommand::Traverse(args) => traverse(config, format, args).await,
        ToolCommand::Path(args) => path(config, format, args).await,
        ToolCommand::Hubs(args) => hubs(config, format, args).await,
        ToolCommand::Stats => stats(config, format).await,
    }
}

async fn store(config: &Config, format: OutputFormat, args: StoreArgs) -> Result<()> {
    if args.content.trim().is_empty() {
        bail!("content must not be blank");
    }
    if args.summary.trim().is_empty() {
        bail!("summary must not be blank");
    }

    let relations = relations(args.relations);
    let attrs = args.attrs.value();
    let store = open_store(config, true)
        .await?
        .expect("create mode must open a store");

    let embedding = embedding(config);
    let (content, vector) = embedding.embed_owned(args.content).await?;
    let ids = store
        .insert_batch(vec![BatchInsertItem {
            content,
            embedding: vector,
            summary: args.summary,
            tags: args.tags,
            context: args.context,
            confidence: args.confidence.unwrap_or(1.0),
            decay_rate: args.decay_rate.unwrap_or(0.0),
            memory_type: args.memory_type,
            attrs,
            relations,
            refs: args.refs,
        }])
        .await?;
    write_value(format, json!({"ids": ids}))
}

async fn search(config: &Config, format: OutputFormat, args: SearchArgs) -> Result<()> {
    if args.query.trim().is_empty() {
        bail!("query must not be blank");
    }
    let include_content = args.include_content.unwrap_or(false);
    let Some(store) = open_store(config, false).await? else {
        return write_memories(format, Value::Array(Vec::new()), "memories: 0");
    };
    let filters = args.filters.options(None);
    let embedding = embedding(config);
    let (query, vector) = embedding.embed_owned(args.query).await?;
    let results = store
        .search(&query, &vector, filters, args.limit.unwrap_or(10).min(100))
        .await?;
    let text = render_search_results(&results, include_content);
    write_memories(format, search_json(&results, include_content), &text)
}

async fn list(config: &Config, format: OutputFormat, args: ListArgs) -> Result<()> {
    if args.ids.len() > 100 {
        bail!("too many identifiers (maximum 100)");
    }
    let include_content = args.include_content.unwrap_or(true);
    let Some(store) = open_store(config, false).await? else {
        return write_memories(format, Value::Array(Vec::new()), "memories: 0");
    };
    let ids = (!args.ids.is_empty()).then_some(args.ids.as_slice());
    let filters = args.filters.options(ids);
    let limit = args
        .limit
        .unwrap_or_else(|| ids.map_or(20, |values| values.len() as u32))
        .min(100);
    let offset = args.offset.unwrap_or(0);
    if include_content {
        let memories = store.list(filters, limit, offset).await?;
        let text = render_list_results(&memories, true);
        write_memories(format, list_json(&memories, true), &text)
    } else {
        let memories = store.list_compact(filters, limit, offset).await?;
        let text = render_compact_list_results(&memories);
        write_memories(format, compact_list_json(&memories), &text)
    }
}

async fn tags(config: &Config, format: OutputFormat) -> Result<()> {
    let Some(store) = open_store(config, false).await? else {
        return write_value(format, json!({}));
    };
    let value = Value::Object(
        store
            .tags()
            .await?
            .into_iter()
            .map(|(tag, count)| (tag, Value::from(count)))
            .collect(),
    );
    write_value(format, value)
}

async fn delete(config: &Config, format: OutputFormat, args: DeleteArgs) -> Result<()> {
    let deleted = match open_store(config, false).await? {
        Some(store) => store.delete(args.id).await?,
        None => false,
    };
    write_value(format, json!({"deleted": deleted, "id": args.id}))
}

async fn update(config: &Config, format: OutputFormat, mut args: UpdateArgs) -> Result<()> {
    if args.clear_tags && !args.tags.is_empty() {
        bail!("--clear-tags conflicts with --tag");
    }
    if args.clear_attrs && !args.attrs.is_empty() {
        bail!("--clear-attrs conflicts with typed attribute flags");
    }
    if args.clear_relations && !args.relations.is_empty() {
        bail!("--clear-relations conflicts with --relation");
    }
    if args.clear_refs && !args.refs.is_empty() {
        bail!("--clear-refs conflicts with --ref");
    }
    if args
        .content
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("content must not be blank");
    }
    if args
        .summary
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("summary must not be blank");
    }

    let has_attrs = args.clear_attrs || !args.attrs.is_empty();
    let has_update = args.content.is_some()
        || args.summary.is_some()
        || args.clear_tags
        || !args.tags.is_empty()
        || args.context.is_some()
        || args.confidence.is_some()
        || args.decay_rate.is_some()
        || args.memory_type.is_some()
        || has_attrs
        || args.clear_relations
        || !args.relations.is_empty()
        || args.clear_refs
        || !args.refs.is_empty();
    if !has_update {
        bail!("at least one update field must be provided");
    }

    let attrs = if args.clear_attrs {
        Some(MemoryAttrs::default())
    } else if has_attrs {
        Some(args.attrs.value())
    } else {
        None
    };
    let tags = if args.clear_tags {
        Some(Vec::new())
    } else if args.tags.is_empty() {
        None
    } else {
        Some(args.tags)
    };
    let relation_values = if args.clear_relations {
        Some(Vec::new())
    } else if args.relations.is_empty() {
        None
    } else {
        Some(relations(args.relations))
    };
    let refs = if args.clear_refs {
        Some(Vec::new())
    } else if args.refs.is_empty() {
        None
    } else {
        Some(args.refs)
    };

    let Some(store) = open_store(config, false).await? else {
        bail!("no memory found with identifier {}", args.id);
    };
    if !store.contains(args.id).await? {
        bail!("no memory found with identifier {}", args.id);
    }

    let mut vector = None;
    if let Some(content) = args.content.take() {
        let service = embedding(config);
        let (content, embedded) = service.embed_owned(content).await?;
        args.content = Some(content);
        vector = Some(embedded);
    }

    let updated = store
        .update(
            args.id,
            UpdateOptions {
                content: args.content.as_deref(),
                summary: args.summary.as_deref(),
                tags: tags.as_deref(),
                embedding: vector.as_deref(),
                context: args.context.as_deref(),
                confidence: args.confidence,
                decay_rate: args.decay_rate,
                memory_type: args.memory_type,
                attrs,
                relations: relation_values.as_deref(),
                refs: refs.as_deref(),
            },
        )
        .await?;
    if !updated {
        bail!("no memory found with identifier {}", args.id);
    }
    write_value(format, json!({"updated": true, "id": args.id}))
}

async fn revalidate(config: &Config, format: OutputFormat, args: RevalidateArgs) -> Result<()> {
    let Some(store) = open_store(config, false).await? else {
        return write_value(format, json!({"revalidated": 0}));
    };
    let count = store.revalidate_batch(&args.ids).await?;
    write_value(format, json!({"revalidated": count}))
}

async fn neighbors(config: &Config, format: OutputFormat, args: NeighborsArgs) -> Result<()> {
    let Some(store) = open_store(config, false).await? else {
        return write_value(format, json!({"outgoing": [], "incoming": []}));
    };
    let result = store.neighbors(args.id, args.kind.as_deref()).await?;
    write_value(format, neighbors_json(result))
}

async fn traverse(config: &Config, format: OutputFormat, args: TraverseArgs) -> Result<()> {
    let depth = args.depth.unwrap_or(2);
    if !(1..=5).contains(&depth) {
        bail!("depth must be between 1 and 5");
    }
    let Some(store) = open_store(config, false).await? else {
        return write_value(
            format,
            json!({"start_id": args.start_id, "nodes": [], "edges": []}),
        );
    };
    let direction = args.direction.unwrap_or_default();
    let result = store
        .traverse(args.start_id, depth, args.kind.as_deref(), direction)
        .await?;
    write_value(format, traverse_json(result))
}

async fn path(config: &Config, format: OutputFormat, args: PathArgs) -> Result<()> {
    let max_hops = args.max_hops.unwrap_or(6);
    if !(1..=10).contains(&max_hops) {
        bail!("max-hops must be between 1 and 10");
    }
    let Some(store) = open_store(config, false).await? else {
        return write_value(format, json!({"found": false, "path": []}));
    };
    let result = store.path(args.from_id, args.to_id, max_hops).await?;
    write_value(format, path_json(result))
}

async fn hubs(config: &Config, format: OutputFormat, args: HubsArgs) -> Result<()> {
    let top_n = args.top_n.unwrap_or(10);
    if !(1..=50).contains(&top_n) {
        bail!("top-n must be between 1 and 50");
    }
    let Some(store) = open_store(config, false).await? else {
        return write_value(format, json!([]));
    };
    write_value(format, hubs_json(store.hubs(top_n).await?))
}

async fn stats(config: &Config, format: OutputFormat) -> Result<()> {
    let Some(store) = open_store(config, false).await? else {
        return write_value(
            format,
            json!({
                "total_memories": 0,
                "by_type": {},
                "total_relations": 0,
                "relation_kinds": [],
            }),
        );
    };
    write_value(format, stats_json(store.graph_stats().await?))
}

async fn open_store(config: &Config, create: bool) -> Result<Option<MemoryStore>> {
    let project_path = config.resolve_project_path(&[])?;
    let db_path = config.db_path_for(&project_path);
    if !create && !db_path.exists() {
        return Ok(None);
    }
    let model = config.embedding_model;
    let fastembed_model = model.into();
    let dimensions = fastembed::TextEmbedding::get_model_info(&fastembed_model)
        .context("configured embedding model is not supported")?
        .dim;
    let store = MemoryStore::open(&db_path, &model.to_string(), dimensions).await?;
    Ok(Some(store))
}

fn embedding(config: &Config) -> EmbeddingService {
    EmbeddingService::new(
        config.embedding_model,
        config.embedding_cache_dir(),
        config.embedding_max_batch_size,
    )
}

fn relations(values: Vec<Relation>) -> Vec<(i64, String)> {
    values
        .into_iter()
        .map(|relation| (relation.id, relation.kind))
        .collect()
}

fn write_memories(format: OutputFormat, json: Value, text: &str) -> Result<()> {
    match format {
        OutputFormat::Text => write_text(text),
        OutputFormat::Json => write_value(format, json),
    }
}

fn write_value(format: OutputFormat, value: Value) -> Result<()> {
    let text = match format {
        OutputFormat::Text => render_toon(value)?,
        OutputFormat::Json => serde_json::to_string_pretty(&value)?,
    };
    write_text(&text)
}

fn write_text(text: &str) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    output.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_serve_and_tool_namespaces() {
        assert!(matches!(
            Cli::try_parse_from(["memlite", "serve"]).unwrap().command,
            Command::Serve
        ));
        assert!(matches!(
            Cli::try_parse_from(["memlite", "tool", "list"])
                .unwrap()
                .command,
            Command::Tool(_)
        ));
    }

    #[test]
    fn parses_global_options_after_nested_subcommands() {
        let cli = Cli::try_parse_from([
            "memlite",
            "tool",
            "list",
            "--format",
            "json",
            "--project-path",
            "/tmp/project",
        ])
        .unwrap();
        assert_eq!(cli.config.explicit_project_path(), Some("/tmp/project"));
    }

    #[test]
    fn parses_and_validates_closed_vocabularies() {
        for (flag, value) in [
            ("--memory-type", "project"),
            ("--status", "resolved"),
            ("--severity", "high"),
            ("--priority", "urgent"),
            ("--outcome", "accepted"),
            ("--phase", "active"),
        ] {
            assert!(Cli::try_parse_from(["memlite", "tool", "list", flag, value]).is_ok());
            assert!(Cli::try_parse_from(["memlite", "tool", "list", flag, "unknown"]).is_err());
            assert!(Cli::try_parse_from(["memlite", "tool", "update", "1", flag, value]).is_ok());
            assert!(
                Cli::try_parse_from(["memlite", "tool", "update", "1", flag, "unknown"]).is_err()
            );
        }
        assert!(
            Cli::try_parse_from(["memlite", "tool", "traverse", "1", "--direction", "both"])
                .is_ok()
        );
        assert!(
            Cli::try_parse_from(["memlite", "tool", "traverse", "1", "--direction", "inn"])
                .is_err()
        );
    }

    #[test]
    fn parses_and_validates_relations() {
        let relation = "42:related_to".parse::<Relation>().unwrap();
        assert_eq!(relation.id, 42);
        assert_eq!(relation.kind, "related_to");
        assert!("0:related_to".parse::<Relation>().is_err());
        assert!("42:".parse::<Relation>().is_err());
    }
}
