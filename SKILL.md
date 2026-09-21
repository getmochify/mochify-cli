---
name: mochify
description: Use this skill whenever the user wants to compress, convert, resize, crop, or rotate images, give a photo an HDR gain map, or work with PDFs (compress a PDF, extract its images, render its pages, split it, or build one from images). Triggers on requests like "compress this image", "convert to WebP/AVIF/JXL", "resize to X pixels wide", "make this HDR", "this PDF is too big to email", "get the photos out of this PDF", "turn these scans into one PDF". Calls the mochify `squish`, `pdf` and `pdf_create` tools with the appropriate parameters.
---

# Mochify — Image & PDF Processing

Use the `squish` tool for images, `pdf` for anything that takes a PDF in, `pdf_create` to build a PDF from images, and `check_usage` to report remaining quota — all via the mochify.app API.

## `squish` parameters

- **file_path** (required) — absolute path to the image file
- **type** — output format: `jpg`, `png`, `webp`, `avif`, `jxl`
- **width** — target width in pixels (height scales proportionally unless `height` is also set)
- **height** — target height in pixels
- **crop** — set `true` to crop to exact `width`×`height` rather than letterboxing
- **rotation** — degrees: `0`, `90`, `180`, `270`
- **output_dir** — directory to write the result (defaults to same directory as input)
- **remove_background** — `true` to cut the subject out; pair with **background** (`"white"`, `"#ff0000"`) to composite, or omit it for transparency
- **quality** — `1`–`100`. Leave it out for automatic selection, which is usually right.
- **smart_compress** — `true` lets saliency pick the quality: detailed subjects keep more, flat areas less. Ignored when **quality** is set.
- **brightness** — `-100` (darkest) to `100` (brightest), for "brighten this" / "it's too dark"
- **optimize_for_web** — `true` for progressive encoding + 4:2:0 chroma, the smallest file to serve
- **lossless** — `true` for pixel-exact output. Only `jxl`, `webp` and `png` can hold it, so set **type** to one of those. The output is usually *larger* than the input, and an already-lossy source (JPEG, AVIF, HEIC) comes back as the best lossy encode instead.
- **hdr** — `"preserve"` keeps a gain map the source already has; `"generate"` also creates one for an SDR source. Use `"generate"` for "make this HDR". Only `jpg` output can carry a gain map, so set **type** to `jpg` unless the user asked for something else.

## Format guidance

| Goal | Recommended format |
|---|---|
| Web photos | `avif` or `webp` |
| Lossless / transparency | `png` |
| Maximum compression | `jxl` |
| Broad compatibility | `jpg` |

## PDF tools

`pdf` takes one PDF and an **op**:

- **optimize** — recompress the images inside it and hand back a smaller PDF, text and layout untouched. This is the one for "compress this PDF". Tune with **quality** (default 75) and **dpi** (the resolution to target for the images kept inside: 96 screen, 150 general, 300 print).
- **extract** — pull out the images somebody placed into the document. **type** defaults to `original` (no re-encode); name a format only if the user did. **max_width** caps each one.
- **rasterize** — render every page to an image. **type** (default `png`) and **dpi** (default 150; 300 for print).
- **split** — one single-page PDF per page. Takes no other parameters.

`optimize` saves a `.pdf`; the others save a `.zip`.

`pdf_create` takes **file_paths** (images, in page order) and builds a PDF: **page** (`fit` default, `a4`, `letter`), **quality**, **combine** (`false` for one PDF per image, returned as a zip).

## Tips

- If the user doesn't specify a format, default to `avif` for photos and `png` for images with transparency.
- If the user says "resize" without a format, keep the original format.
- For "web-optimised" or "compress for web" requests, use `avif` at the user's desired width (or 1200px if unspecified), with **optimize_for_web**.
- Don't set **quality** unless the user asked for a specific quality or a visibly smaller/better file — automatic selection beats a guess.
- Multiple files can be processed in sequence with separate `squish` calls; `pdf_create` takes all its images in one call.
- "Extract the images" means `pdf` with op `extract`; "convert the pages to images" means op `rasterize`. "Split" with an image format named is `rasterize`, not `split`.
- Always confirm the output path back to the user after processing.
