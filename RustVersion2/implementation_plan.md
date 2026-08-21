# Comic Converter Rust Refactor Implementation Plan

## Goal
Rewrite the C# WPF Comic Converter application into a standalone Rust application to drastically reduce the executable size from ~25MB (framework-dependent + Magick) / 150MB (self-contained) down to a ~5-8MB true standalone `.exe`.

## Proposed UI Framework
We will use **egui (eframe)**. It is an immediate-mode GUI that compiles purely to machine code, requiring zero external dependencies like WebView2, making it perfect for a tiny, portable utility.

## Architectural Changes (C# to Rust mappings)

### 1. Concurrency and Main Workflow
- **C#**: `Parallel.ForEach(tasks, parallelOptions, ...)`
- **Rust**: Use the `rayon` crate for trivial data-parallelism (`tasks.par_iter().for_each(|task| { ... })`).
- *Impact*: Will match or exceed current parallelism performance effortlessly.

### 2. File and Archive Extraction
- **C#**: `System.IO.Compression.ZipFile` and `Process.Start` for 7z.exe.
- **Rust**: 
  - Standard Zips: `zip` crate for native extraction.
  - 7z Fallback: Use `std::process::Command::new` to trigger the fallback `7z` executable identically to the C# workflow.

### 3. Image Processing (Replacing Magick.NET)
- **Library**: We will rely heavily on the `image` crate.
- **Resizing**: `image::imageops::resize(..., FilterType::Lanczos3)`.
- **Formatting & Quality**: The `webp` crate encoding logic.
- **Smart Trim**: Completely ported by manually iterating over the image buffer (`img.get_pixel(x,y)`) identically to the current `CalculateSmartTrimBounds`. We will compute bounding boxes based on user-defined percentage tolerances and use `image::imageops::crop` to slice the final image.

## Open Questions for the Next Chat
> [!IMPORTANT]
> When you open the new chat, let me know if you would prefer **Tauri** instead of **egui** if you want the app to look like a standard web website rather than a custom tool interface! 

## Phase Breakdown for New Chat
1. **Init Project**: `cargo new comic_converter` and setup `Cargo.toml`.
2. **Build Core Logic**: Write the Rust module for filesystem traversal, extraction, and archiving.
3. **Build Image Engine**: Migrate the `CalculateSmartTrimBounds` and WebP generation logic.
4. **Build UI (egui)**: Replicate the checkboxes, text fields, and progress bars.
5. **Final Polish & Release**: Build with `cargo build --release` utilizing `lto = true` and `opt-level = "z"` to aggressively strip it to the smallest possible `.exe`.
