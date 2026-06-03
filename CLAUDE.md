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

# Process a PDF (auto-detected by .pdf extension)
cargo run -- document.pdf --op rasterize -t png --dpi 150
cargo run -- document.pdf -p "split into pngs"   # NLP prompt path

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
                   - no subcommand     → calls process_files(), which routes
                     `.pdf` inputs to process_pdfs() and images to the squish flow
  cli.rs         Clap `Args` struct and `Commands` enum (Serve subcommand)
  api.rs         `MochifyClient` + `ProcessParams` / `PdfParams` — all HTTP logic.
                   `squish()` posts an image to /v1/squish; `pdf()` posts a PDF to
                   /v1/pdf and saves the returned zip. Response bytes written to disk.
  mcp/
    mod.rs       Re-exports MochifyMcp
    tools.rs     `MochifyMcp` struct implements ServerHandler via rmcp macros.
                   Exposes `squish` (mirrors ProcessParams) and `pdf` (mirrors PdfParams).
```

### Key design decisions

- **Thin tools in MCP mode** — the MCP client (e.g. Claude) handles natural-language interpretation and maps prompts to the structured `squish` / `pdf` tool parameters. No NLP layer needed in the CLI (the CLI's own `--prompt` flag does call `/v1/prompt`, including `mode: "pdf"` for PDFs).
- **PDFs are auto-detected** by the `.pdf` extension on the default path; PDFs and images can't be mixed in one invocation (the NLP prompt resolves to a single mode). `--op split|rasterize` (plus `--dpi`, `-t`, `--quality` for rasterize) configure it, or `--prompt` resolves it. Output is saved as a `.zip`.
- **Auth is optional** — without `--api-key` / `MOCHIFY_API_KEY`, requests go through on the free tier (25/month; unauthenticated IPs get 3/month). The key is sent as `Authorization: Bearer <key>`.
- **rmcp macros pattern** — tools use `#[tool_router]` on the impl block + `#[tool_handler]` on `impl ServerHandler`. The struct must have a `tool_router: ToolRouter<Self>` field initialized via `Self::tool_router()`.

### API wire format

`POST /v1/squish` — raw image bytes as the body, params in the query string:

| Parameter | Query param | Type |
|---|---|---|
| Image file | request body | raw bytes (`Content-Type: image/*`) |
| Format | `type` | `jpg \| png \| webp \| avif \| jxl` |
| Width | `width` | u32 |
| Height | `height` | u32 |
| Crop | `crop` | bool |
| Rotation | `rotate` | 0 / 90 / 180 / 270 |

`POST /v1/pdf` — raw PDF bytes (`Content-Type: application/pdf`) as the body; returns a zip:

| Parameter | Query param | Type |
|---|---|---|
| PDF file | request body | raw bytes |
| Operation | `op` | `split` (per-page PDFs) \| `rasterize` (page images) |
| Format | `type` | rasterize only: `png \| jpg \| webp` |
| DPI | `dpi` | rasterize only: u32 (default 150) |
| Quality | `quality` | rasterize only: 1–100 (lossy formats) |

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
