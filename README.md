# mochify-cli

A command-line tool and MCP server for [mochify.xyz](https://mochify.xyz) — a fast, privacy-first image compression and conversion API powered by a native C++ engine.

Compress and convert images to modern formats (AVIF, JXL, WebP, Jpegli) from your terminal, or give AI assistants like Claude direct access to image processing via the [Model Context Protocol](https://modelcontextprotocol.io).

## Installation

Download the latest binary from [Releases](https://github.com/getmochify/mochify-cli/releases), or build from source:

```bash
cargo install --path .
```

```bash
cargo build --release
# binary at ./target/release/mochify
```

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
| `-k, --api-key <KEY>` | API key (or set `MOCHIFY_API_KEY` env var) |

### Examples

```bash
# Convert a JPEG to AVIF
mochify photo.jpg -t avif

# Resize and convert to WebP
mochify photo.jpg -t webp -w 800

# Batch convert a folder to AVIF at 1200px wide
mochify ./images/*.jpg -t avif -w 1200 -o ./compressed

# With an API key
MOCHIFY_API_KEY=your-key mochify photo.jpg -t jxl

# Use a natural-language prompt instead of explicit flags
mochify photo.jpg -p "convert to avif and resize to 1200px wide"

# Prompt works with multiple files too
mochify ./images/*.jpg -p "compress for web, keep under 1000px wide" -o ./out
```

Free usage is 25 images per day without an API key. Visit [mochify.xyz](https://mochify.xyz) for more.

## MCP Server (Claude Desktop)

`mochify` can run as an [MCP server](https://modelcontextprotocol.io), letting Claude process images on your behalf directly from conversation.

Available on [Smithery](https://smithery.ai/server/@getmochify/mochify-cli) for one-click install.

### Setup

Add the following to your Claude Desktop config at `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "mochify": {
      "command": "/path/to/mochify-cli",
      "args": ["serve"],
      "env": {
        "MOCHIFY_API_KEY": "your-key-here"
      }
    }
  }
}
```

Restart Claude Desktop. The mochify server will appear in your connections.

### Usage

Just describe what you want in natural language, with the full path to your image:

> "Convert `/Users/me/Desktop/photo.jpg` to AVIF at 1000px wide"

> "Compress all the JPEGs in `/Users/me/projects/blog/images/` to WebP and save them to `/Users/me/projects/blog/compressed/`"

Claude will call the `squish` tool automatically.

## API

Powered by the mochify.xyz API at `https://api.mochify.xyz/v1/squish`.

- Images are processed in-memory and never stored on disk
- Supports JPEG (Jpegli), AVIF, JXL, WebP, and PNG output
- Up to 25MB per image

Visit [mochify.xyz](https://mochify.xyz) for the web interface.
