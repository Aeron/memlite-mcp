# Client setup

MemLite runs as a local stdio MCP server. There is no MemLite client plugin to
install. The examples below use built-in MCP support, with a direct CLI fallback
for older Pi versions.

Upstream documentation and source were checked on October 31, 2026.

## Shared setup

Build the binary with `cargo build --release --locked` from the MemLite repository.
The executable is `target/release/memlite`.

Replace `/absolute/path/to/memlite` in the examples with the absolute path to
your built executable. If you installed the binary on `PATH`, you can use
`memlite` instead. Merge entries into existing configuration files rather
than replacing other settings.

Register `memlite serve`, not the binary alone. Clients with separate command
and argument fields need `"args": ["serve"]`. Clients with a command array include
`"serve"` after the executable.

By default, MemLite uses the first usable local `file://` root reported by the
MCP client, then the server's current working directory. Launch clients from
the project directory when they do not provide roots. Direct CLI commands use
the current working directory. Clients that resolve to the same project path
share its database.

If automatic selection does not identify the project you want, use the optional
`--project-path` flag or `MEMLITE_PROJECT_PATH` environment variable. The flag
takes precedence over the variable, and both take precedence over MCP roots
and the working directory. An explicit override skips the roots request. This
can also select a stable identity for a multi-root workspace. See
[project identity](configuration.md#project-identity) for resolution details.

## Claude Code and Pi coding agent

Both clients accept this JSON shape:

```json
{
  "mcpServers": {
    "memlite": {
      "command": "/absolute/path/to/memlite",
      "args": ["serve"]
    }
  }
}
```

### Claude Code

Save the entry in `.mcp.json` at the project root. Start Claude Code and approve
the project server when prompted. Check the connection with `claude mcp list` or
`/mcp` inside a session.

For private registration without a project file, run this from the target project:

```sh
claude mcp add --scope local --transport stdio memlite -- \
  /absolute/path/to/memlite serve
```

Claude Code stores local-scope registrations in `~/.claude.json` under the project
path. Its user scope loads across projects. The
[Claude Code MCP reference][claude] documents the schema, scopes, and commands.

### Pi coding agent

Pi added native MCP support in [version 0.99][pi-release]. Save the JSON above in
`.pi/mcp.json` and trust the project. The user-level file
`~/.pi/agent/mcp.json` applies across projects. Run `pi mcp list` to check the
connection. Run `/reload` after changing configuration in an active session,
then inspect `/mcp`.

Pi's per-server `timeout` uses seconds and defaults to 60. Add `"timeout": 300`
inside the `memlite` object if the first embedding call needs more time. An
installed extension that replaces `/mcp` also replaces Pi's native session MCP
support, so its configuration may differ. See [Pi's MCP documentation][pi].

On Pi versions without native MCP, ask the agent to run MemLite's direct CLI
through its [built-in `bash` tool][pi-bash]. Run these commands from the project
directory with `memlite` on `PATH`:

```sh
memlite tool --format json list --limit 10 --include-content false
memlite tool --format json search "project conventions" --limit 5
```

This route does not use MCP and needs no bridge. The singular `tool` namespace
provides all memory operations. See the [CLI reference](configuration.md#tool).

## Codex

Add this table to `.codex/config.toml` in a trusted project. Codex also reads
user-level settings from `~/.codex/config.toml`.

```toml
[mcp_servers.memlite]
command = "/absolute/path/to/memlite"
args = ["serve"]
tool_timeout_sec = 300
```

Run `codex mcp list` or `/mcp` in the Codex terminal interface to check the server.
Codex defaults to 10 seconds for startup and 60 seconds for tool calls. The
example increases the tool timeout for cold model loading. See the
[Codex MCP reference][codex].

## OpenCode 2

Add this entry to `opencode.jsonc` at the project root. The global file is
`~/.config/opencode/opencode.jsonc`. Both locations also accept `.json` files.

```json
{
  "mcp": {
    "servers": {
      "memlite": {
        "type": "local",
        "command": ["/absolute/path/to/memlite", "serve"]
      }
    }
  }
}
```

Run `opencode mcp list` or open `/mcps` in a session. OpenCode 2 uses
`mcp.servers` for its native schema. Its per-server `timeout` is an object with
`startup`, `catalog`, and `execution` fields in milliseconds. Defaults are
30 seconds for startup and catalog requests, and 12 hours for execution. See the
[OpenCode 2 MCP reference][opencode2] and [configuration paths][opencode2-config].
Use the next example for OpenCode 1.

## Kilo Code and OpenCode 1

Current Kilo Code and OpenCode 1 use a server map directly under `mcp`:

```json
{
  "mcp": {
    "memlite": {
      "type": "local",
      "command": ["/absolute/path/to/memlite", "serve"],
      "timeout": 300000
    }
  }
}
```

For Kilo Code, put the entry in project-root `kilo.jsonc` or
`.kilo/kilo.jsonc`. Its global file is `~/.config/kilo/kilo.jsonc`. Current VS Code,
CLI, and JetBrains clients share these files. Open Settings, then MCP to inspect
the server in VS Code, or run `kilo mcp list`. See
[Kilo's MCP setup][kilo] and [shared settings paths][kilo-settings].

For OpenCode 1, use project-root `opencode.jsonc` or `opencode.json`. Its global
file is `~/.config/opencode/opencode.json`. Run `opencode mcp list` to check the
server. See the [OpenCode 1 MCP reference][opencode1] and
[configuration paths][opencode1-config].

These schemas use a numeric `timeout` in milliseconds, unlike OpenCode 2's
native timeout object. The example sets five minutes. Kilo's
[request timeout selection][kilo-source] applies the server value to tool calls.

## Hermes agent

Add this entry to `mcp_servers` in `~/.hermes/config.yaml`, or in the active
Hermes profile's `config.yaml`:

```yaml
mcp_servers:
  memlite:
    command: "/absolute/path/to/memlite"
    args: ["serve"]
    timeout: 300
    connect_timeout: 60
```

Hermes uses seconds for both timeout fields. `timeout` limits tool calls and
defaults to 300 seconds in current source. `connect_timeout` defaults to
60 seconds and bounds the initial connection. These are separate limits. See the
[configuration reference][hermes-reference],
[tool timeout source][hermes-timeout], and [connection timeout source][hermes-source].

Restart Hermes or run `/reload-mcp` after editing the configuration. Run
`hermes mcp test memlite` to check the connection. Standard Hermes installations
include MCP support. See the [Hermes MCP guide][hermes].

## Zed

Add this entry to the project's `.zed/settings.json` or Zed's user settings file
at `~/.config/zed/settings.json` on macOS and Linux.

```json
{
  "context_servers": {
    "memlite": {
      "command": "/absolute/path/to/memlite",
      "args": ["serve"],
      "timeout": 300
    }
  }
}
```

Keep `command` as a string alongside `args`. Do not nest these fields
inside a command object. Zed's tool timeout uses seconds and defaults to 60.
The example sets five minutes.

Run `agent: open settings` and open MCP Servers to inspect the connection.
These settings configure Zed Agent. Terminal threads use the selected CLI's own
MCP configuration. See [Zed's MCP documentation][zed],
[settings paths][zed-settings], and [timeout schema][zed-source].

## First use

Ask the agent to call MemLite's `memory_stats` after connecting. MemLite exposes
12 `memory_*` tools. Clients may prefix their names. A new project reports zero
memories.

The first embedding operation downloads the selected model if it is not cached.
Server startup and `memory_stats` do not trigger that download. The first
`memory_store` or `memory_search` call can therefore take longer than later calls.
Raise the client's tool timeout if that operation expires; increasing only the
startup timeout will not help. Claude Code's per-server `timeout` uses
milliseconds, so five minutes is `300000`, not `300`. See
[Claude Code's time limits][claude-timeouts] and
[MemLite's embedding configuration](configuration.md#embedding-models).

[claude]: https://code.claude.com/docs/en/mcp
[claude-timeouts]: https://code.claude.com/docs/en/mcp#time-limits-on-mcp-tool-calls
[codex]: https://developers.openai.com/codex/mcp/
[pi]: https://pi.dev/docs/latest/mcp
[pi-release]: https://pi.dev/changelog/releases/0.99.0
[pi-bash]: https://pi.dev/docs/latest/shell-aliases
[opencode2]: https://opencode.ai/v2/docs/mcp-servers
[opencode2-config]: https://opencode.ai/v2/docs/config
[opencode1]: https://opencode.ai/docs/mcp-servers/
[opencode1-config]: https://opencode.ai/docs/config/
[kilo]: https://kilo.ai/docs/automate/mcp/using-in-kilo-code
[kilo-settings]: https://kilo.ai/docs/getting-started/settings
[kilo-source]: https://github.com/Kilo-Org/kilocode/blob/main/packages/opencode/src/mcp/index.ts
[hermes]: https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp
[hermes-reference]: https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference
[hermes-timeout]: https://github.com/NousResearch/hermes-agent/blob/main/tools/mcp_tool_common.py
[hermes-source]: https://github.com/NousResearch/hermes-agent/blob/main/tools/mcp_tool.py
[zed]: https://zed.dev/docs/ai/mcp
[zed-settings]: https://github.com/zed-industries/zed/blob/main/docs/src/configuring-zed.md
[zed-source]: https://github.com/zed-industries/zed/blob/main/crates/settings_content/src/project.rs
