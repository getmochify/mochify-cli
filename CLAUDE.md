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

# Ultra HDR gain map (jpg output only)
cargo run -- photo.jpg -t jpg --hdr generate

# Quality controls
cargo run -- photo.jpg -t webp -q 70
cargo run -- photo.jpg -t avif --smart-compress --optimize-for-web
cargo run -- scan.png -t webp --lossless

# Process a PDF (auto-detected by .pdf extension)
cargo run -- document.pdf --op rasterize -t png --dpi 150
cargo run -- report.pdf --op optimize -q 75 --dpi 150
cargo run -- document.pdf -p "split into pngs"   # NLP prompt path

# Build a PDF from images (--op create; images in, PDF out)
cargo run -- page-*.jpg --op create --page a4

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
                     `--op create` to create_pdf() (images in, PDF out),
                     `.pdf` inputs to process_pdfs(), and images to the squish flow
  cli.rs         Clap `Args` struct and `Commands` enum (Serve subcommand)
  api.rs         `MochifyClient` + `ProcessParams` / `PdfParams` — all HTTP logic.
                   `squish()` posts an image to /v1/squish; `pdf()` posts a PDF to
                   /v1/pdf; `pdf_create()` posts images to /v1/pdf?op=create as
                   multipart. `PdfParams::for_op()` validates the op and maps the
                   generic dpi/max-width inputs onto that op's query parameters.
                   Response bytes written to disk.
  mcp/
    mod.rs       Re-exports MochifyMcp
    tools.rs     `MochifyMcp` struct implements ServerHandler via rmcp macros.
                   Exposes `squish` (mirrors ProcessParams), `pdf` (the PDF-in ops)
                   and `pdf_create` (images → PDF).
```

### Key design decisions

- **Thin tools in MCP mode** — the MCP client (e.g. Claude) handles natural-language interpretation and maps prompts to the structured `squish` / `pdf` tool parameters. No NLP layer needed in the CLI (the CLI's own `--prompt` flag does call `/v1/prompt`, including `mode: "pdf"` for PDFs).
- **PDFs are auto-detected** by the `.pdf` extension on the default path; PDFs and images can't be mixed in one invocation (the NLP prompt resolves to a single mode). `--op optimize|extract|rasterize|split` configures it, or `--prompt` resolves it (worker NLP `mode: "pdf"`). `optimize` returns a `.pdf`, the rest return a `.zip`.
- **`--op create` is chosen by flag, not extension** — its inputs are images, so there is nothing to route on. All files go up in one multipart request (`images` parts, in order) and come back as one PDF, or a zip of one-page PDFs with `--no-combine`. Its prompt mode is `imgpdf`.
- **One `--dpi` and one `--max-width` across all five PDF ops.** That is how people think about it ("what resolution", "how wide"), while the API spells them differently per op (`dpi` vs `maxDpi`, `maxWidth` vs `maxDimension`). `PdfParams::for_op()` owns that mapping, so the CLI and the MCP tools cannot drift apart. The web app maps its NLP output the same way.
- **`-q/--quality` serves both paths.** An invocation is either images or PDFs, never both, so a second quality flag would only be a second thing to get wrong — same reasoning as the single `--dpi`.
- **Local pre-flight on `--lossless`.** The API 400s `lossless` with `jpg`/`avif`; `api::LOSSLESS_FORMATS` lets the CLI and the MCP tool refuse it without spending a request. `X-Mochify-Lossless: downgraded` (an already-lossy source) is reported back, since the request could not be honoured literally.
- **HDR is a squish param** (`--hdr preserve|generate`, `hdr=1|generate` on the wire). The NLP returns a plain boolean, which maps to `generate` — the mode that also synthesises a gain map for an SDR source, which is what someone asking for HDR in words means. Only `jpg` output can carry one, so the CLI reads `X-Mochify-HDR` back and says so when the output carries none.
- **Batches run concurrently, but report in order.** `run_batch()` in `main.rs` keeps `-j/--jobs` requests in flight (default 4) across both the image and PDF loops, which previously awaited one round trip per file. A finished job waits in its slot until every job submitted before it has been printed, so stdout still matches the order the files were given — `mochify *.jpg > list.txt` and a piped `find` both depend on that. One job per (file, variant): a prompt that answers a single file with two formats overlaps the same way two files do.
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
| Clarity | `clarity` | bool |
| Remove background | `removeBackground` | bool |
| Background colour | `background` | string |
| Metadata | `stripExif` | bool (default true) |
| HDR gain map | `hdr` | `1` (preserve) \| `generate` (also synthesise) |
| Quality | `quality` | 1–100 (default: auto) |
| Smart compression | `smartCompress` | bool |
| Brightness | `brightness` | -100..=100 |
| Web delivery | `optimizeForWeb` | bool |
| Lossless | `lossless` | bool — `jxl` \| `webp` \| `png` only |

Response headers worth reading: `X-Mochify-HDR` (`true` \| `generated` \| `false`), `X-Mochify-Lossless` (`true` \| `downgraded`), `X-Mochify-Optimized`, `X-Mochify-Quality`, `X-Mochify-Saliency`, `X-Mochify-BgRemoved`.

`POST /v1/pdf` — raw PDF bytes (`Content-Type: application/pdf`) as the body, except `op=create`, which takes `multipart/form-data` with each image appended as `images`:

| Op | Body | Returns | Query params |
|---|---|---|---|
| `optimize` | PDF | PDF | `quality` (75), `maxDpi` (150), `maxDimension` (2000), `minSize` (64) |
| `extract` | PDF | zip | `type` (`original` \| png \| jpg \| webp \| avif \| jxl), `quality` (82), `maxWidth` (0), `minSize` (64) |
| `rasterize` | PDF | zip | `type` (png), `dpi` (150, 36–300), `quality` (82) |
| `split` | PDF | zip | none |
| `create` | images (multipart) | PDF, or zip when `combine=0` | `page` (`fit` \| a4 \| letter), `quality` (82), `dpi` (96), `maxWidth` (0), `combine` (1) |

Response headers: `X-Mochify-Saved-Pct` and `X-Mochify-Images-Recompressed` (optimize), `X-Mochify-Pages` (rasterize/split/create), `X-Mochify-Images` (extract).

Plan gating: the PDF-in ops need a paid plan; `create` is on every plan, including Free.

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
