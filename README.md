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
| `-p, --prompt <TEXT>` | Natural-language prompt — resolves all params automatically |
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

### Output file naming

By default, when the output format and directory match the input, the result is saved as `{name}_mochified.{ext}` so it's always clear something happened. If that file already exists, a numeric suffix is added (`_1`, `_2`, etc.). When the format changes (e.g. `.jpg` → `.webp`), the extension change is already unambiguous so no suffix is added.

Use `-n, --name` to set an explicit base name: `mochify photo.jpg -t webp -n hero` saves `hero.webp`. The prompt path also supports this: `mochify *.jpg -p "optimise for Shopify, name them product"` will produce `product.webp`, `product_1.webp`, etc.

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

Claude calls the `squish` tool automatically and reports back the saved path and file size.

## API

Powered by `https://api.mochify.app/v1/squish`. Images are processed in-memory and never written to disk.

| Plan | Ops/month | Max file size |
|---|---|---|
| Free (no account) | 3/batch | 20 MB |
| Free (with account) | 25 | 20 MB |
| Seller ($7.99/mo) | 300 | 75 MB |
| Pro ($24.99/mo) | 1,200 | 75 MB |

Visit [mochify.app](https://mochify.app) for the web interface, pricing, and API docs.
