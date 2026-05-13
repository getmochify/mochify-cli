# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`mochify-cli` is a Rust CLI tool and MCP server that wraps the [mochify.app](https://mochify.app) image processing API (`POST https://api.mochify.app/v1/squish`). It uploads local images via multipart form and saves the processed result.

## Commands

```bash
# Build
cargo build

# Build release
cargo build --release

# Check (fast, no binary output)
cargo check

# Run (process an image)
cargo run -- photo.jpg -t webp -w 800

# Run MCP server on stdio
cargo run -- serve

# Run tests
cargo test

# Run a single test
cargo test <test_name>

# Lint
cargo clippy

# Format
cargo fmt
```

## Architecture

```
src/
  main.rs        Async entry point. Parses CLI args (clap), dispatches:
                   - `serve` subcommand → starts MCP server on stdio
                   - no subcommand     → calls process_files()
  cli.rs         Clap `Args` struct and `Commands` enum (Serve subcommand)
  api.rs         `MochifyClient` + `ProcessParams` — all HTTP logic.
                   `squish()` builds a multipart POST, writes response bytes to disk.
  mcp/
    mod.rs       Re-exports MochifyMcp
    tools.rs     `MochifyMcp` struct implements ServerHandler via rmcp macros.
                   Exposes a single `squish` tool whose schema mirrors ProcessParams.
```

### Key design decisions

- **Single tool in MCP mode** — the MCP client (e.g. Claude) handles natural-language interpretation and maps prompts to structured `squish` tool parameters. No NLP layer needed in the CLI.
- **Auth is optional** — without `--api-key` / `MOCHIFY_API_KEY`, requests go through on the free tier (25/day). The key is sent as `Authorization: Bearer <key>`.
- **rmcp macros pattern** — tools use `#[tool_router]` on the impl block + `#[tool_handler]` on `impl ServerHandler`. The struct must have a `tool_router: ToolRouter<Self>` field initialized via `Self::tool_router()`.

### API wire format

| Parameter | Form field | Type |
|---|---|---|
| Image file | `file` | multipart bytes |
| Format | `type` | `jpg \| png \| webp \| avif \| jxl` |
| Width | `width` | u32 |
| Height | `height` | u32 |
| Crop | `crop` | bool |
| Rotation | `rotation` | 0 / 90 / 180 / 270 |

### MCP config (Claude Desktop)

```json
{
  "mcpServers": {
    "mochify": {
      "command": "/path/to/mochify-cli",
      "args": ["serve"],
      "env": { "MOCHIFY_API_KEY": "your-key" }
    }
  }
}
```
