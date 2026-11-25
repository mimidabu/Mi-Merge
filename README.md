# Mi-Merge

A lightweight, fast Windows command-line tool written in Rust that merges video clips end to end into a single MP4 file—preserving audio, adding one chapter per clip, and fitting vertical clips into a landscape frame with black pillarboxing (never stretched or cropped).

## Features

- **End-to-end merging** with no trimming, editing, or transitions.
- **Optional chapters**: off by default; enable one chapter per clip (titled with the clip's file name without extension via FFMETADATA `-map_chapters`) with `--chapters` or `MI_MERGE_CHAPTERS=1`.
- **Smart fast path**: when all input clips already match resolution, FPS, pixel format, codec, SAR, rotation, and audio format, Mi-Merge uses FFmpeg's `concat` demuxer with `-c copy` (lossless and near-instant).
- **Automatic pillarboxing**: when clips differ (e.g., mixed vertical and landscape phone videos, different FPS, or clips without audio), Mi-Merge builds a single `filter_complex` that scales with `force_original_aspect_ratio=decrease`, pads black bars, normalizes `setsar=1`, target FPS, `yuv420p`, and 48 kHz stereo audio (injecting `anullsrc` silence for silent clips).
- **Natural sorting**: pass files or entire folders—folder contents are automatically sorted in natural order (`clip2.mp4` before `clip10.mp4`).
- **Hardware acceleration**: defaults to `libx264` (`veryfast`, CRF 20, AAC 192k, `+faststart`), with `--encoder` support for `nvenc`, `qsv`, and `amf`.
- **Single portable binary**: compiles to a standalone `mi-merge.exe` and locates `ffmpeg`/`ffprobe` via CLI flag, next to `mi-merge.exe`, or on `PATH`.

## Windows Setup

### 1. Install FFmpeg

Install FFmpeg (includes `ffmpeg.exe` and `ffprobe.exe`) via `winget`:

```powershell
winget install Gyan.FFmpeg
```

Restart your terminal so `ffmpeg` and `ffprobe` are available on `PATH`.

> **Portable alternative:** Download a Windows FFmpeg build and place `ffmpeg.exe` and `ffprobe.exe` in the same folder as `mi-merge.exe`, or pass `--ffmpeg C:\path\to\ffmpeg.exe`.

### 2. Build `mi-merge.exe`

Install Rust (if you don't already have it):

```powershell
winget install Rustlang.Rustup
```

Clone and build the release binary:

```powershell
git clone https://github.com/mimidabu/mi-merge.git
cd mi-merge
cargo build --release
```

The compiled executable will be at `target\release\mi-merge.exe`. Copy it anywhere on your `PATH` (or alongside `ffmpeg.exe` and `ffprobe.exe`).

## Usage

```text
mi-merge [OPTIONS] <INPUT>... -o <FILE>
```

### Examples

Merge individual clips:

```powershell
mi-merge intro.mp4 main.mp4 outro.mp4 -o merged.mp4
```

Merge all video clips in a folder (naturally sorted) and overwrite existing output:

```powershell
mi-merge .\vacation-clips -o vacation.mp4 -y
```

Preview the merge plan and exact FFmpeg command without running it:

```powershell
mi-merge .\clips -o merged.mp4 --dry-run
```

Force 1080p 60fps using NVIDIA NVENC:

```powershell
mi-merge .\clips --resolution 1920x1080 --fps 60 --encoder nvenc -o merged.mp4 -y
```

Specify a custom FFmpeg location:

```powershell
mi-merge .\clips --ffmpeg C:\tools\ffmpeg\bin\ffmpeg.exe -o merged.mp4
```

### Options

| Flag | Description |
| --- | --- |
| `<INPUT>...` | One or more video files or folders (folders are expanded in natural sort order). |
| `-o, --output <FILE>` | Output video file path (e.g. `merged.mp4`). |
| `-y, --overwrite` | Overwrite output file if it already exists. |
| `--dry-run` | Print the merge plan, chapter list, and FFmpeg command without executing. |
| `--chapters` | Add one chapter per clip (off by default; can also be enabled via `MI_MERGE_CHAPTERS=1`). |
| `--resolution <WxH>` | Override target output resolution (e.g. `1920x1080`). Defaults to first landscape clip (else first clip). |
| `--fps <FPS>` | Override target output frame rate (e.g. `30`, `60`, `29.97`, `30000/1001`). Defaults to first landscape clip (else first clip). |
| `--encoder <ENCODER>` | Video encoder when transcoding: `x264` (default), `nvenc`, `qsv`, `amf`. |
| `--ffmpeg <PATH>` | Path to `ffmpeg` executable (or directory containing `ffmpeg` and `ffprobe`). |
| `--ffprobe <PATH>` | Path to `ffprobe` executable. |

## How FFmpeg Discovery Works

Mi-Merge looks for `ffmpeg` and `ffprobe` in this order:

1. Explicit `--ffmpeg` / `--ffprobe` flags (if `--ffmpeg` is given, its folder is also checked for `ffprobe`).
2. Next to `mi-merge.exe`.
3. System `PATH`.

## Running Tests

Unit tests and end-to-end integration tests (which generate synthetic video clips via FFmpeg `lavfi` and verify output streams and chapters with `ffprobe`):

```powershell
cargo test
```

## License

MIT
