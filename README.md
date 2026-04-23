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

This opens your browser, where you sign in and authorize the CLI. Your credentials are saved automatically to `~/.config/mochify/credentials.toml` — no environment variables or manual key copying required. Both the CLI and MCP server pick them up automatically.

```bash
mochify auth status   # check whether you're signed in
mochify auth logout   # remove saved credentials
```

Free tier (unauthenticated) is limited to 25 images per day. Sign up at [mochify.app](https://mochify.app).

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
| `--crop` | Crop to exact dimensions |
| `-r, --rotation <DEG>` | Rotation: `0`, `90`, `180`, `270` |
| `-o, --output <DIR>` | Output directory (default: same as input) |
| `-p, --prompt <TEXT>` | Natural-language prompt — resolves params automatically |
| `-k, --api-key <KEY>` | API key override (or set `MOCHIFY_API_KEY` env var) |

### Examples

```bash
# Convert a JPEG to AVIF
mochify photo.jpg -t avif

# Resize and convert to WebP
mochify photo.jpg -t webp -w 800

# Batch convert a folder to AVIF at 1200px wide
mochify ./images/*.jpg -t avif -w 1200 -o ./compressed

# Use a natural-language prompt instead of explicit flags
mochify photo.jpg -p "convert to avif and resize to 1200px wide"

# Prompt works with multiple files too
mochify ./images/*.jpg -p "compress for web, keep under 1000px wide" -o ./out
```

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

Just describe what you want in natural language, with the full path to your image:

> "Convert `/Users/me/Desktop/photo.jpg` to AVIF at 1000px wide"

> "Compress all the JPEGs in `/Users/me/projects/blog/images/` to WebP and save them to `/Users/me/projects/blog/compressed/`"

Claude will call the `squish` tool automatically.

## API

Powered by the mochify API at `https://api.mochify.xyz/v1/squish`.

- Images are processed in-memory and never stored on disk
- Supports JPEG (Jpegli), AVIF, JXL, WebP, and PNG output
- Up to 25MB per image

Visit [mochify.app](https://mochify.app) for the web interface.
