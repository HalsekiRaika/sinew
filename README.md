# sinew

Peer discovery and messaging for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) sessions.

sinew enables multiple Claude Code instances to discover each other and exchange messages in real-time through a lightweight local broker daemon.

> Inspired by [louislva/claude-peers-mcp](https://github.com/louislva/claude-peers-mcp). Rewritten from scratch in Rust as a single, portable binary.

**[Japanese / 日本語](README.ja.md)**

## Features

- **Peer Discovery** - Find other Claude Code sessions on the same machine, directory, or git repository
- **Real-time Messaging** - Send and receive messages between sessions via channel notifications
- **Auto Broker** - The broker daemon launches automatically when needed
- **Single Binary** - No runtime dependencies. One executable for both broker and MCP server
- **Cross-platform** - Windows, macOS, and Linux support

## Installation

### From source

```bash
cargo install --git https://github.com/HalsekiRaika/sinew
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/HalsekiRaika/sinew/releases), or use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall sinew --git https://github.com/HalsekiRaika/sinew
```

## Quick Start

### 1. Configure Claude Code

Add sinew to your Claude Code MCP settings (`~/.claude/claude_desktop_config.json` or equivalent):

```json
{
  "mcpServers": {
    "sinew": {
      "command": "sinew",
      "args": ["serve"]
    }
  }
}
```

That's it. The broker starts automatically when the first session connects.

### 2. Use from Claude Code

Once configured, Claude Code gains four tools:

| Tool | Description |
|------|-------------|
| `list_peers` | Discover other Claude Code sessions |
| `send_message` | Send a message to another session |
| `check_messages` | Check for incoming messages |
| `set_summary` | Set a work summary visible to peers |

### 3. Channel Notifications

When a message arrives, sinew pushes a real-time notification to Claude Code via `notifications/claude/channel`. To enable this during the research preview:

```bash
claude --dangerously-load-development-channels server:sinew
```

The `server:sinew` argument specifies which MCP server is allowed to send channel notifications. Without this flag, messages can still be retrieved manually using `check_messages`.

## Architecture

```
Claude Code A                         Claude Code B
     |                                      |
  [MCP Server]                         [MCP Server]
  (sinew serve)                        (sinew serve)
     |                                      |
     +----------> [Broker Daemon] <---------+
                  (sinew broker)
                  localhost:7899
                      |
                   [SQLite]
```

sinew uses a two-process design:

- **Broker** - HTTP server on `localhost:7899` with SQLite storage. Central registry for peers and message router.
- **MCP Server** - One per Claude Code session. Communicates with the broker via HTTP. Sends heartbeats every 15s and polls for messages every 1s.

## CLI

```
sinew <COMMAND>

Commands:
  broker    Start the Broker daemon
  serve     Start the MCP server (stdio transport)
  shutdown  Shutdown the running Broker daemon
  status    Show status of the Broker and connected peers
```

### `sinew broker`

Start the broker daemon manually (usually not needed - `serve` auto-launches it).

```bash
sinew broker --port 7899
```

### `sinew serve`

Start the MCP server. This is what Claude Code calls.

```bash
sinew serve --broker-url http://127.0.0.1:7899
```

### `sinew status`

Check broker health and connected peer count.

```bash
sinew status
# Broker: ok (http://127.0.0.1:7899)
# Peers:  3
```

### `sinew shutdown`

Gracefully stop the broker daemon.

```bash
sinew shutdown
```

## Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter (e.g., `debug`, `info`, `warn`) |

### Defaults

| Setting | Value |
|---------|-------|
| Broker port | `7899` |
| Broker URL | `http://127.0.0.1:7899` |
| Heartbeat interval | 15 seconds |
| Message poll interval | 1 second |
| Database location | `{TEMP_DIR}/sinew-broker.db` |

## Peer Scopes

When listing peers, you can filter by scope:

| Scope | Returns |
|-------|---------|
| `machine` | All peers on the system |
| `directory` | Peers in the same working directory |
| `repo` | Peers in the same git repository |

## Building from Source

Requires Rust 1.85+ (edition 2024).

```bash
git clone https://github.com/HalsekiRaika/sinew.git
cd sinew
cargo build --release
```

### Running Tests

```bash
cargo test
```

### Lint

```bash
cargo clippy
cargo deny check
```

## Acknowledgements

This project is a ground-up Rust rewrite inspired by the design of [claude-peers-mcp](https://github.com/louislva/claude-peers-mcp) by [@louislva](https://github.com/louislva).
