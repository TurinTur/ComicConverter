# Comic Converter (Rust)

A fast, cross-platform desktop application for batch-converting comic archives
(CBZ, ZIP, CBR, RAR, 7z or plain folders of images) into size-optimized CBZ
files with WebP-encoded pages.

It is a Rust rewrite of the original C#/.NET (WPF) version, which is archived
in the [.net](./.net) folder. The Rust version uses the same workflow and
options but is a single self-contained executable with no .NET runtime
dependency.

## What it does

For every source item (archive file or folder), the app:

1. **Extracts** the archive into a temp working folder
   (ZIP/CBZ use the built-in zip handling; CBR/RAR/7z require an optional
   path to `7z.exe`, which runs silently in the background).
2. **Replicates** the folder structure in a target folder.
3. **Converts** every page image to **WebP**, in parallel using all configured
   threads:
   - optional **trim**: crops uniform borders around each page;
   - optional **smart trim**: also crops when small elements such as page
     numbers interrupt an otherwise uniform border;
   - optional **resize** by percentage (Lanczos3 filter);
   - configurable WebP quality (default 67).
4. **Archives** the results back to CBZ — one single CBZ for everything,
   one CBZ per folder/file, or no zipping at all (keep folder structure).
5. Optionally **copies** the final CBZ files to the final output folder,
   **deletes the source** if the result is smaller, and **cleans up** the
   temp folder.

A log panel shows each step live, including initial/final sizes, the achieved
size reduction and the total processing time. All settings are persisted to
`settings.json` next to the executable.

Typical result: a 259 MB input converted to a 77 MB CBZ (~56–70% smaller).

## Installation

### Option A: download / build the release binary

No runtime dependencies are needed. Build from source (see below) or grab
`target/release/comic_converter.exe`.

### Option B: build from source

Requirements:

- [Rust](https://rustup.rs) (stable)
- On Windows, either the MSVC Build Tools or a MinGW toolchain
  (`x86_64-pc-windows-gnu`)

```sh
cargo build --release
```

The executable is written to `target/release/comic_converter.exe`.

> Note: on Windows the release build uses the `windows` subsystem, so no
> console window appears when launching the GUI.

## Usage

### GUI

Run `comic_converter.exe` (no arguments). Then:

1. **Sources** – add files and/or folders with *Add Files* / *Add Folder*,
   remove individual items with *X*, or clear the list.
2. **Folders**
   - *Temp Folder*: where extraction and conversion happen.
   - *Final Folder*: where the final CBZ files are copied.
   - *7z.exe Path (optional)*: only needed for CBR/RAR/7z inputs; leave empty
     to use the built-in ZIP handling.
3. **Options** (hover any control for a tooltip):
   - *Delete Source if final size is smaller*
   - *Delete Temp Folder at end*
   - *Copy final zip to Final Output Folder*
   - *Trim pages (remove borders)*
   - *Smart Trim (ignore small elements like page numbers)*
   - *Include volume/chapter range in name*
4. **Trim Settings** – min size after trim (default 75%), smart threshold
   (default 97%) and color tolerance (default 8).
5. **Conversion Settings** – number of parallel threads (defaults to CPU
   count), WebP quality 1–100 (default 67), resize percentage (100 = keep
   original size).
6. **Zip Mode** – *Single* (one CBZ for all), *Individual* (one CBZ per
   folder/file) or *None* (keep folder structure).
7. Press **Start** and watch the progress bar and log.

Settings are saved automatically on start and on exit.

### Headless benchmark mode

For scripted/timed runs without the GUI:

```sh
comic_converter.exe --benchmark <sources...> [--temp <dir>] [--final <dir>] \
    [--7z <path>] [--threads <n>] [--resize <%>] ...
```

Run `comic_converter.exe --benchmark` with no sources to print all options.

## Technical overview

```
src/
├── main.rs         GUI (egui/eframe), styling, settings wiring, worker thread
├── processor.rs    Conversion pipeline orchestration
├── image_engine.rs Per-image pipeline: trim → resize → WebP encode
├── benchmark.rs    Headless CLI benchmark runner
└── settings.rs     AppSettings struct + JSON persistence (+ unit tests)
```

### main.rs

- Builds the UI with [egui](https://github.com/emilk/egui) via `eframe`
  (immediate-mode GUI): left options panel, central sources/folders/log area,
  bottom action bar with progress bar.
- Applies a custom dark theme (accent colors, corner radii, text styles).
- On *Start*, validates inputs, clones the settings into a
  `ProcessorContext`, spawns a worker thread and receives
  `ProgressReport { percentage, message }` updates over an
  `std::sync::mpsc` channel, which are polled each frame into the log and
  progress bar. The channel also guards against a dead worker so the UI never
  stays locked.
- The log buffer is capped (~512 KB) so long runs don't grow memory
  indefinitely.

### processor.rs

`ComicProcessor::process` drives the six-step pipeline described above:

- Size accounting before/after conversion, reported as MB and % reduction.
- Extraction is parallelized with [rayon](https://github.com/rayon-rs/rayon);
  7z failures are detected via exit status (never silently ignored).
- 7z child processes are started with the Windows `CREATE_NO_WINDOW`
  flag (0x08000000), so no console windows flash during parallel extraction.
- Image conversion runs through rayon with per-file error collection and
  periodic "Converted N of M files" progress reports.
- When *Copy Final* is off, archives are written next to the temp folder so
  they aren't destroyed by the temp cleanup.
- Volume/chapter ranges can be parsed from names and included in output file
  names (regex-based).

### image_engine.rs

Single pass over each page held as one `RgbaImage`:

1. decode via the `image` crate and convert to RGBA once;
2. trim / smart trim by scanning border rows/columns whose pixels match the
   background within a tolerance (percentage-based threshold); the crop is
   only applied if the result keeps at least `min_size` of each dimension;
3. optional Lanczos3 resize;
4. encode with the `webp` crate at the configured quality.

Already-WebP pages can be copied through without re-encoding.

### benchmark.rs

A CLI-only mode (`--benchmark`) that reuses the same engines to measure
per-phase timings (extract / convert / archive) for performance work, without
initializing any GUI.

### settings.rs

`AppSettings` is a plain serde struct serialized to pretty JSON
(`settings.json`, stored next to the executable). Unknown/partial files fall
back to per-field defaults thanks to `#[serde(default)]`; unit tests cover
round-trips and partial-file loading.

### Dependencies

| Crate      | Purpose                                  |
|------------|------------------------------------------|
| eframe/egui| GUI framework                            |
| rfd        | native file/folder dialogs               |
| image      | decoding, cropping, resizing             |
| webp       | WebP encoding                            |
| zip        | ZIP/CBZ read & write                     |
| walkdir    | recursive directory traversal            |
| rayon      | data parallelism                         |
| regex      | volume/chapter range parsing             |
| serde(+json)| settings persistence                    |
| chrono     | log timestamps                           |
| num_cpus   | default thread count                     |
