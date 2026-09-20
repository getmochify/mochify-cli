# mochify-cli

[![MCP Badge](https://lobehub.com/badge/mcp/getmochify-mochify-cli)](https://lobehub.com/mcp/getmochify-mochify-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

<a href="https://glama.ai/mcp/servers/@getmochify/mochify-mcp">
  <img width="380" height="200" src="https://glama.ai/mcp/servers/@getmochify/mochify-mcp/badge" />
</a>

A command-line tool and MCP server for [mochify.app](https://mochify.app) — a fast, privacy-first image compression and conversion API powered by a native C++ engine.

Compress and convert images to modern formats (AVIF, JXL, WebP, Jpegli) from your terminal, or give AI assistants like Claude direct access to image processing via the [Model Context Protocol](https://modelcontextprotocol.io).

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
```

Outputs are named after the input: `report_compressed.pdf`, `brochure_images.zip`, `document_rasterized.zip`, `document_pages.zip`, and `<first image>.pdf` for `create` (override with `-n`).

`optimize`, `extract`, `rasterize` and `split` require a paid plan; `create` works on every plan, including Free.

## MCP Server (Claude Desktop)

`mochify` can run as an [MCP server](https://modelcontextprotocol.io), letting Claude process images on your behalf directly from conversation.

### Setup

Run `mochify auth login` first, then add the following to your Claude Desktop config at `~/Library/Application Support/Claude/claude_desktop_config.json`:

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

Restart Claude Desktop. The mochify server will appear in your connections and use your saved credentials automatically.

### Usage

Describe what you want in natural language with the full path to your image:

> "Convert `/Users/me/Desktop/photo.jpg` to AVIF at 1000px wide"

> "Compress all the JPEGs in `/Users/me/projects/blog/images/` to WebP and save to `/Users/me/projects/blog/compressed/`"

> "Optimise `/Users/me/Desktop/product.jpg` for eBay"

> "Remove the background from `/Users/me/Desktop/shirt.png` and save as WebP"

> "Rasterize `/Users/me/Desktop/report.pdf` to PNGs at 200 DPI"

> "Compress `/Users/me/Desktop/report.pdf` — it's too big to email"

> "Make `/Users/me/Desktop/sunset.jpg` HDR"

> "Brighten `/Users/me/Desktop/dim.jpg` a bit and optimise it for my website"

> "Turn the scans in `/Users/me/Desktop/receipts/` into one A4 PDF"

Claude calls the `squish` tool for images, the `pdf` tool for anything that takes a PDF in (optimize, extract, rasterize, split), and `pdf_create` to build a PDF from images, and reports back the saved path and file size.

## API

Powered by `https://api.mochify.app` — `/v1/squish` for images and `/v1/pdf` for PDF optimize/extract/rasterize/split/create. Files are processed in-memory and never written to disk.

| Plan | Ops/month | Max file size |
|---|---|---|
| Free (no account) | 3/batch | 20 MB |
| Free (with account) | 25 | 20 MB |
| Seller ($7.99/mo) | 300 | 75 MB |
| Pro ($24.99/mo) | 1,200 | 75 MB |

Visit [mochify.app](https://mochify.app) for the web interface, pricing, and API docs.
