# Client setup

MemLite runs as a local stdio MCP server. Configure your client to launch
`memlite serve`, not the executable alone. No client plugin is needed.

## Shared setup

The commands below assume a Homebrew installation. They register the binary's
absolute path because clients may not inherit your interactive shell's `PATH`.
Config snippets use `/opt/homebrew/bin/memlite`; replace it with the result of
`brew --prefix` followed by `/bin/memlite`, or your binary's installed path.
JSON, TOML, and YAML do not evaluate shell substitutions.

Choose either the command or the config-file option for your client. Merge
snippets into existing files rather than replacing other settings. The examples
use user-level configuration so you do not need to repeat setup for each project.

MemLite selects a project from MCP workspace roots, then the server's working
directory. Launch clients from the project directory if they do not provide roots.
Clients resolving to the same project path share its database. For explicit
selection or a stable multi-root identity, use `--project-path` or
`MEMLITE_PROJECT_PATH`. See [project identity](configuration.md#project-identity).

## Claude Code

Register with the CLI:

```sh
claude mcp add --transport stdio --scope user memlite -- "$(brew --prefix)/bin/memlite" serve
```

Or merge this entry into `~/.claude.json`:

```json
{
  "mcpServers": {
    "memlite": {
      "command": "/opt/homebrew/bin/memlite",
      "args": ["serve"]
    }
  }
}
```

Start a new session, then check `claude mcp list` or `/mcp`.
See the [Claude Code MCP reference][claude].

## Codex

Register with the CLI:

```sh
codex mcp add memlite -- "$(brew --prefix)/bin/memlite" serve
```

Or add this table to `~/.codex/config.toml`:

```toml
[mcp_servers.memlite]
command = "/opt/homebrew/bin/memlite"
args = ["serve"]
```

Restart Codex or start a new task, then check `codex mcp list` or `/mcp`.
See the [Codex MCP reference][codex].

## Pi

Register with the CLI:

```sh
pi mcp add memlite -- "$(brew --prefix)/bin/memlite" serve
```

Or merge this entry into `~/.pi/agent/mcp.json`:

```json
{
  "mcpServers": {
    "memlite": {
      "command": "/opt/homebrew/bin/memlite",
      "args": ["serve"]
    }
  }
}
```

Run `/reload` in an active session, then check `pi mcp list` or `/mcp`.
See [Pi's MCP documentation][pi].

## OpenCode 2

Register with the CLI:

```sh
opencode mcp add memlite --global -- "$(brew --prefix)/bin/memlite" serve
```

Or merge this entry into `~/.config/opencode/opencode.jsonc`:

```json
{
  "mcp": {
    "servers": {
      "memlite": {
        "type": "local",
        "command": ["/opt/homebrew/bin/memlite", "serve"]
      }
    }
  }
}
```

OpenCode 2 uses `mcp.servers`, unlike the server map directly under `mcp` in
OpenCode 1 and Kilo. Start a new session, then check `opencode mcp list` or `/mcps`.
See [MCP settings][opencode2] and [config paths][opencode2-config].

## Kilo Code

Register with the CLI:

```sh
kilo mcp add memlite -- "$(brew --prefix)/bin/memlite" serve
```

Or merge this entry into `~/.config/kilo/kilo.json` (also accepts `.jsonc`):

```json
{
  "mcp": {
    "memlite": {
      "type": "local",
      "command": ["/opt/homebrew/bin/memlite", "serve"]
    }
  }
}
```

Start a new session, then check `kilo mcp list`. In VS Code, open Settings, then
MCP. See [Kilo MCP setup][kilo] and [shared settings][kilo-settings].

## OpenCode 1

For guided setup, run `opencode mcp add`, choose the global config and local
server type, name it `memlite`, and enter `/opt/homebrew/bin/memlite serve`.

Or merge this entry into `~/.config/opencode/opencode.json`:

```json
{
  "mcp": {
    "memlite": {
      "type": "local",
      "command": ["/opt/homebrew/bin/memlite", "serve"]
    }
  }
}
```

Start a new session, then check `opencode mcp list`.
See [OpenCode 1 MCP setup][opencode1] and [config paths][opencode1-config].

## Hermes

Register with the CLI and review the discovered tools when prompted:

```sh
hermes mcp add memlite --command "$(brew --prefix)/bin/memlite" --args serve
```

Or merge this entry into `~/.hermes/config.yaml` or your active profile's
`config.yaml`:

```yaml
mcp_servers:
  memlite:
    command: "/opt/homebrew/bin/memlite"
    args: ["serve"]
```

Run `/reload-mcp` in an active session, then check `hermes mcp test memlite`.
See the [Hermes MCP guide][hermes] and [CLI reference][hermes-cli].

## Zed

Open `agent: open settings`, then MCP Servers, and choose Add Server, then Add
Local Server. Set the command to `/opt/homebrew/bin/memlite` and add `serve` as
an argument.

Or merge this entry into `~/.config/zed/settings.json`:

```json
{
  "context_servers": {
    "memlite": {
      "command": "/opt/homebrew/bin/memlite",
      "args": ["serve"]
    }
  }
}
```

Check that MemLite shows as active in MCP Servers. This configures Zed Agent;
terminal threads use their CLI's MCP configuration. See [Zed MCP setup][zed]
and [settings paths][zed-settings].

## First use

Ask the agent to call `memory_stats` to check the connection. A new project
reports zero memories. MemLite exposes 12 `memory_*` tools; clients may prefix
their names. See the [tool reference](tools.md).

The first operation that needs embeddings (`memory_store` or `memory_search`)
downloads the selected model if it is not cached. Download time depends on your
internet connection. Default tool timeouts, including Zed's 60 seconds, may
already be sufficient; increase yours only if that call times out. Server startup
and `memory_stats` do not load the model. See
[embedding configuration](configuration.md#embedding-models).

[claude]: https://code.claude.com/docs/en/mcp
[codex]: https://developers.openai.com/codex/mcp/
[pi]: https://pi.dev/docs/latest/mcp
[opencode2]: https://opencode.ai/v2/docs/mcp-servers
[opencode2-config]: https://opencode.ai/v2/docs/config
[opencode1]: https://opencode.ai/v1/docs/mcp-servers/
[opencode1-config]: https://opencode.ai/v1/docs/config/
[kilo]: https://kilo.ai/docs/automate/mcp/using-in-kilo-code
[kilo-settings]: https://kilo.ai/docs/getting-started/settings
[hermes]: https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp
[hermes-cli]: https://hermes-agent.nousresearch.com/docs/reference/cli-commands
[zed]: https://zed.dev/docs/ai/mcp
[zed-settings]: https://github.com/zed-industries/zed/blob/main/docs/src/configuring-zed.md
