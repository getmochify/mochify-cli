# mochify-cli

[![MCP Badge](https://lobehub.com/badge/mcp/getmochify-mochify-cli)](https://lobehub.com/mcp/getmochify-mochify-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

<a href="https://glama.ai/mcp/servers/@getmochify/mochify-mcp">
  <img width="380" height="200" src="https://glama.ai/mcp/servers/@getmochify/mochify-mcp/badge" />
</a>

A command-line tool and MCP server for [mochify.app](https://mochify.app) — a fast, privacy-first image compression and conversion API powered by a native C++ engine.

Compress and convert images to modern formats (AVIF, JXL, WebP, Jpegli) from your terminal, work on PDFs, or give AI assistants like Claude direct access to the same engine over the [Model Context Protocol](https://modelcontextprotocol.io) — either as a local stdio server (`mochify serve`) or the hosted one at [`mcp.mochify.app`](https://mcp.mochify.app), which needs no install at all.

**Jump to:** [Installation](#installation) · [CLI usage](#cli-usage) · [PDF processing](#pdf-processing) · [MCP server](#mcp-server) · [Plans](#api)

## Installation

**macOS (Homebrew):**

```bash
brew tap getmochify/mochify
brew install mochify
```

**Windows (Scoop):**

```powershell
scoop bucket add mochify https://github.com/getmochify/scoop-mochify
scoop install mochify
```

**Linux / WSL:**

```bash
# x86_64
curl -L https://github.com/getmochify/mochify-cli/releases/latest/download/mochify-linux-x86_64 -o mochify
chmod +x mochify
sudo mv mochify /usr/local/bin/

# arm64
curl -L https://github.com/getmochify/mochify-cli/releases/latest/download/mochify-linux-arm64 -o mochify
chmod +x mochify
sudo mv mochify /usr/local/bin/
```

**Cargo (all platforms):**

```bash
cargo install mochify
```

Requires [Rust](https://rustup.rs). Easiest option on Linux/WSL if you already have the toolchain.

**Manual:** All binaries at [Releases](https://github.com/getmochify/mochify-cli/releases).

**From source:**

```bash
cargo install --path .
```

## Authentication

Sign in with your [mochify.app](https://mochify.app) account to unlock your full quota:

```bash
mochify auth login
```

This opens your browser, where you sign in and authorize the CLI. Credentials are saved automatically to `~/.config/mochify/credentials.toml` — no environment variables or manual key copying required. Both the CLI and MCP server pick them up automatically.

```bash
mochify auth status   # check whether you're signed in
mochify auth logout   # remove saved credentials
```

Without an account you get 3 images per batch (IP-based). With a free account: 25 images/month. Sign up at [mochify.app](https://mochify.app).

## CLI Usage

```bash
mochify [OPTIONS] <FILES>...
```

### Options

| Flag | Description |
|---|---|
| `-t, --type <FORMAT>` | Output format: `jpg`, `png`, `webp`, `avif`, `jxl` |
| `-w, --width <N>` | Target width in pixels |
| `-H, --height <N>` | Target height in pixels |
| `--crop` | Crop to exact dimensions (saliency-guided) |
| `-r, --rotation <DEG>` | Rotation: `0`, `90`, `180`, `270` |
| `-o, --output <DIR>` | Output directory (default: same as input) |
| `-n, --name <NAME>` | Base name for the output file (without extension) |
| `--clarity` | Apply clarity (midtone contrast enhancement — crisper, more detailed look) |
| `--remove-bg` | Remove the background (AI foreground isolation) |
| `--background <COLOR>` | Composite colour for `--remove-bg` (`white`, `#ff0000`, …) |
| `--keep-metadata` | Preserve EXIF/metadata (stripped by default) |
| `-q, --quality <N>` | Output quality `1–100` (default: automatic) |
| `--smart-compress` | Saliency-guided quality — detail keeps more, flat areas less |
| `--brightness <N>` | Exposure, `-100` (darkest) to `+100` (brightest) |
| `--optimize-for-web` | Progressive + 4:2:0 chroma — smallest file to serve |
| `--lossless` | Pixel-exact output (`jxl`, `webp`, `png` only) |
| `--hdr [MODE]` | Ultra HDR gain map: `preserve` (bare flag) or `generate` |
| `-p, --prompt <TEXT>` | Natural-language prompt — resolves all params automatically |
| `-j, --jobs <N>` | Files to process at once (default `4`; `1` for strictly one at a time) |
| `-k, --api-key <KEY>` | API key override (or set `MOCHIFY_API_KEY` env var) |

### Examples

```bash
# Convert to AVIF
mochify photo.jpg -t avif

# Resize and convert to WebP
mochify photo.jpg -t webp -w 800

# Batch convert a folder to AVIF at 1200px wide
mochify ./images/*.jpg -t avif -w 1200 -o ./compressed

# Natural-language prompt — let the AI pick the right params
mochify photo.jpg -p "convert to avif, 1200px wide"
mochify photo.jpg -p "optimise for eBay"
mochify photo.jpg -p "remove background and convert to WebP"
mochify photo.jpg -p "resize to 50%, strip EXIF, keep as WebP"

# Custom output name
mochify photo.jpg -t webp -n hero
mochify product.jpg -p "optimise for Shopify" -n product-main

# Pipe file paths from stdin
find . -name "*.jpg" | mochify -t webp -o ./out
cat images.txt | mochify -p "convert to avif 1200px wide" -o ./compressed
ls *.heic | mochify -t jpg
```

### Quality

Quality is chosen automatically unless you say otherwise.

```bash
# Fixed quality
mochify photo.jpg -t webp -q 70

# Let saliency pick it — detailed subjects keep more, flat areas less
mochify photo.jpg -t avif --smart-compress

# Smallest file to serve from a web server
mochify photo.jpg -t webp --optimize-for-web

# Brighten a dark photo
mochify photo.jpg --brightness 30

# Pixel-exact
mochify scan.png -t webp --lossless
```

`--lossless` only works for `jxl`, `webp` and `png` — `jpg` and `avif` are rejected before the request goes out. It overrides `-q` and `--smart-compress`, and the output is usually **larger** than the input: lossless preserves pixels, not file size. A source that is already lossy (JPEG, AVIF, HEIC) comes back as the best lossy encode instead, since nothing can restore what that file discarded — the CLI says so when that happens.

### HDR (Ultra HDR gain maps)

`--hdr` controls the Ultra HDR / ISO 21496-1 gain map that makes a photo render with real headroom on an HDR display.

| Mode | What it does |
|---|---|
| `--hdr` (or `--hdr preserve`) | Keeps a gain map the source already has. Never invents one, so it does nothing to an SDR photo. |
| `--hdr generate` | Keeps an existing gain map **and** synthesises one when the source is plain SDR. This is the one for "make it HDR". |

Only `jpg` output can carry a gain map (`jxl` carries HDR by a different route; `avif`, `webp` and `png` cannot), so pair it with `-t jpg`. It is also skipped alongside `--clarity` or `--remove-bg`, which change the base the gain map is a ratio to. The CLI reads the `X-Mochify-HDR` response header and tells you when the output ended up with no gain map.

```bash
# Keep the headroom an iPhone photo already captured, converting to JPEG
mochify IMG_1234.heic -t jpg --hdr

# Give an SDR photo a gain map
mochify photo.jpg --hdr generate

# Or just say so
mochify photo.jpg -p "make this HDR"
```

### Output file naming

By default, when the output format and directory match the input, the result is saved as `{name}_mochified.{ext}` so it's always clear something happened. If that file already exists, a numeric suffix is added (`_1`, `_2`, etc.). When the format changes (e.g. `.jpg` → `.webp`), the extension change is already unambiguous so no suffix is added.

Use `-n, --name` to set an explicit base name: `mochify photo.jpg -t webp -n hero` saves `hero.webp`. The prompt path also supports this: `mochify *.jpg -p "optimise for Shopify, name them product"` will produce `product.webp`, `product_1.webp`, etc.

### PDF processing

PDFs are detected automatically by the `.pdf` extension, and `--op` picks what to do with them. (PDFs and images can't be mixed in a single command — run them separately.)

`create` is the exception: its inputs are images, so there is nothing in a `.jpg` to route on. Pass `--op create`, or just say "pdf" in a prompt — `mochify *.jpg -p "convert to pdf"` builds one too.

| Op | Takes | Returns | What it does |
|---|---|---|---|
| `optimize` | a PDF | `.pdf` | Recompresses the images inside the PDF. Text, fonts, vector art and layout are untouched, so the document stays searchable. |
| `extract` | a PDF | `.zip` | Pulls out the images somebody placed into the document, at the resolution they were stored at. |
| `rasterize` | a PDF | `.zip` | Renders every page to an image, text and all. |
| `split` | a PDF | `.zip` | Explodes the PDF into one single-page PDF per page. |
| `create` | images | `.pdf` | Builds a PDF from images, one page per image, in the order given. |

| Flag | Applies to | Description |
|---|---|---|
| `--op <OP>` | all | `optimize`, `extract`, `rasterize`, `split`, `create` |
| `-t, --type <FORMAT>` | rasterize, extract | `png`, `jpg`, `webp`, `avif`, `jxl` — plus `original` for `extract` (no re-encode) |
| `--dpi <N>` | rasterize, optimize, create | Render resolution (rasterize, default `150`), target resolution for the images kept inside the PDF (optimize), or page sizing (create) |
| `-q, --quality <N>` | all but split | Output quality `1–100` (same flag as for images) |
| `--max-width <N>` | extract, optimize, create | Cap image width in pixels; `0` leaves sizes alone |
| `--min-size <N>` | extract, optimize | Skip images smaller than this on either axis; `0` takes everything |
| `--page <SIZE>` | create | `fit` (default), `a4`, `letter` |
| `--no-combine` | create | One single-page PDF per image, returned as a `.zip` |

```bash
# Make a PDF smaller without touching the text
mochify report.pdf --op optimize -q 75 --dpi 150

# Pull the embedded images out as WebP, capped at 1600px
mochify brochure.pdf --op extract -t webp --max-width 1600

# Render pages to PNG at 150 DPI, or high-res JPEGs for print
mochify document.pdf --op rasterize -t png --dpi 150
mochify document.pdf --op rasterize -t jpg --dpi 300 -q 90

# One PDF per page
mochify document.pdf --op split

# Build a PDF from images (one page per image, in the order given)
mochify page-*.jpg --op create --page a4 -n scanned
mochify page-*.jpg --op create --no-combine   # one PDF per image, as a zip

# Or describe it in natural language
mochify report.pdf -p "compress this pdf"
mochify brochure.pdf -p "get the photos out as png"
mochify document.pdf -p "split into pngs"
mochify page-*.jpg --op create -p "one a4 pdf, good quality"

# A prompt that says "pdf" on image inputs implies --op create
mochify page-*.jpg -p "convert to pdf"
mochify scan-*.png -p "one a4 pdf, good quality"
mochify shot-*.jpg -p "each image as its own pdf"
```

Outputs are named after the input: `report_compressed.pdf`, `brochure_images.zip`, `document_rasterized.zip`, `document_pages.zip`, and `<first image>.pdf` for `create` (override with `-n`).

`optimize`, `extract`, `rasterize` and `split` require a paid plan; `create` works on every plan, including Free.

## MCP server

Mochify speaks [MCP](https://modelcontextprotocol.io) two ways. Both call the same engine at `api.mochify.app`, where encoding happens in RAM and the original is discarded immediately. The difference is where your files live.

| | Hosted (`mcp.mochify.app`) | Local (`mochify serve`) |
|---|---|---|
| Install | none | this binary |
| Transport | Streamable HTTP | stdio |
| Auth | OAuth 2.0 + PKCE, one browser approval | `mochify auth login`, or anonymous |
| Input | public HTTPS URL, or base64 bytes | absolute paths on your disk |
| Output | short-lived download URL (~5 min) on `files.mochify.app` | written straight to disk |
| Image bytes through the agent | on base64 input | never |
| Handles | images | images and PDFs |

Reach for **hosted** when the agent has no filesystem — Claude.ai, a hosted runtime, anything talking to images that already live on the web. Reach for **local** when the files are on your machine, which is most of the time: nothing but a path and a byte count ever enters the model's context.

Both can run side by side in the same client with independent tokens.

### Hosted (no install)

Register `https://mcp.mochify.app` as a remote MCP connector and complete the OAuth flow once. No API key to copy or rotate.

- **Claude.ai** — Settings → Connectors → Add custom connector
- **Claude Desktop** — Settings → Connectors, or a `"type": "http"` entry in the config
- **Cursor, Windsurf, Gemini, any client with remote MCP support** — same URL

Or install it from Smithery:

[![smithery badge](https://smithery.ai/badge/mochify/mochify)](https://smithery.ai/servers/mochify/mochify)

Results come back as a download URL with roughly a five-minute expiry, held in an in-memory pickup store until fetched or the TTL lapses. Originals are never stored. This is the one path that is not zero-retention end to end — see [why we relaxed zero retention for MCP](https://mochify.app/guides/why-we-relaxed-zero-retention-for-mcp).

### Local (stdio)

Run `mochify auth login` first, then point your client at the binary:

```json
{
  "mcpServers": {
    "mochify": {
      "command": "mochify",
      "args": ["serve"]
    }
  }
}
```

Claude Desktop's config lives at `~/Library/Application Support/Claude/claude_desktop_config.json` on macOS. Restart the client afterwards; the server picks up your saved credentials automatically. Any stdio MCP client works the same way — Cursor, Continue, Cline, Claude Code.

Set `MOCHIFY_API_KEY` instead for CI, or leave auth off entirely to run on the anonymous free tier.

### Tools

Both servers expose the same four tools. The difference is how files get in and out: the local server takes paths and writes to disk, the hosted one takes URLs or base64 and hands back a short-lived download URL.

| Tool | Takes | Does |
|---|---|---|
| `squish` | one image | Everything on the image side: format conversion, resize, crop, rotate, background removal, brightness, clarity, quality control (fixed, saliency-guided or lossless), web optimisation, Ultra HDR gain maps. |
| `pdf` | one PDF | `optimize` (smaller PDF, text and layout untouched), `extract` (the images inside it, as a zip), `rasterize` (pages to images, as a zip), `split` (one PDF per page, as a zip). |
| `pdf_create` | images | Builds a PDF, one page per image, in the order given. `fit`, `a4` or `letter` pages; `combine: false` returns a zip of single-page PDFs. |
| `check_usage` | nothing | Reports operations remaining this billing period and the plan they belong to. Needs authentication. |

`squish` parameters: `file_path`, `type`, `width`, `height`, `crop`, `rotation`, `quality`, `smart_compress`, `lossless`, `optimize_for_web`, `brightness`, `clarity`, `remove_background`, `background`, `strip_metadata`, `hdr`, `output_dir`, `output_name`.

`pdf` parameters: `file_path`, `op`, `type`, `dpi`, `quality`, `max_width`, `min_size`, `output_dir`.

`pdf_create` parameters: `file_paths`, `page`, `quality`, `dpi`, `max_width`, `combine`, `output_name`, `output_dir`.

`check_usage` takes no parameters.

Every tool reads and writes the filesystem itself, so the agent never has to load an image to process it. Responses carry the saved path plus whatever the API reported: how much smaller the PDF got, whether a gain map survived, whether a lossless request had to fall back to lossy, and how much quota is left.

### Usage

Describe what you want in natural language, with the full path to your file:

> "Convert `/Users/me/Desktop/photo.jpg` to AVIF at 1000px wide"

> "Compress all the JPEGs in `/Users/me/projects/blog/images/` to WebP and save to `/Users/me/projects/blog/compressed/`"

> "Optimise `/Users/me/Desktop/product.jpg` for eBay"

> "Remove the background from `/Users/me/Desktop/shirt.png` and save as WebP"

> "Compress `/Users/me/Desktop/report.pdf` — it's too big to email"

> "Rasterize `/Users/me/Desktop/report.pdf` to PNGs at 200 DPI"

> "Make `/Users/me/Desktop/sunset.jpg` HDR"

> "Brighten `/Users/me/Desktop/dim.jpg` a bit and optimise it for my website"

> "Turn the scans in `/Users/me/Desktop/receipts/` into one A4 PDF"

## API

Powered by `https://api.mochify.app` — `/v1/squish` for images, `/v1/pdf` for PDF optimize/extract/rasterize/split/create. Files are processed in RAM and never written to disk.

| Plan | Ops/month | Files per batch | Max file size |
|---|---|---|---|
| Free (no account) | 3 per batch | 3 | 20 MB |
| Free (with account) | 25 | 3 | 20 MB |
| Day Pass ($2 one-time) | 100, valid 24h | 25 | 75 MB |
| Seller ($7.99/mo) | 300 | 25 | 75 MB |
| Pro ($24.99/mo) | 1,200 | 25 | 75 MB |
| Growth ($79.99/mo) | 5,000 | 25 | 75 MB |

One operation is one output image. Compress, convert, resize and rotate the same file and it stays one operation; ask for two formats at two sizes and that is four. PDF `optimize`, `extract`, `rasterize` and `split` need a paid plan (Day Pass counts); `create` works on every plan including Free.

Visit [mochify.app](https://mochify.app) for the web interface, pricing, and API docs.

## Questions

Ask in [GitHub Discussions](https://mochify.app/community) — answers stay searchable there, which a support email thread does not. Bugs and feature requests belong in [Issues](https://github.com/getmochify/mochify-cli/issues). For anything account or billing related, email [hello@mochify.app](mailto:hello@mochify.app).

Listed in the official MCP Registry as `mcp-name: app.mochify/mochify`.

