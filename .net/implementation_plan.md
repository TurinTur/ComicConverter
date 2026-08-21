# Comic WebP Converter WPF Application

The goal is to convert your PowerShell script `WebPJobPSv2 - copia.ps1` into a standalone C# WPF application for Windows. This application will feature a modern graphical user interface and manage the underlying processing asynchronously while keeping the form responsive.

## Decisions Made

- **ImageMagick**: We will use **Magick.NET** (Q16-AnyCPU) for built-in processing.
- **Archiving**: We will use a C# library (`SharpCompress` or `System.IO.Compression`) for extraction and creation of CBZ files, but we will add an option in the UI to specify a fallback `7z.exe` path if the user prefers external extraction.
- **Git Repository**: Set up remote at `https://github.com/TurinTur/ComicConverter.git`.

## Proposed Changes

### 1. Workspace and Git Setup
- Create project directory at `C:\temp\ComicConverter`.
- Initialize Git repository.
- Commit the initial scaffolding.

### 2. WPF Application Project Initialization
I will create a new WPF project (`ComicWebPConverter`) using .NET 8.0 targeting Windows.

### 3. User Interface (GUI)
The UI will have a clean, easy-to-use form:
- **Folders Selection:** Text boxes with "Browse" buttons for:
  - Source Folder
  - Temp Target Folder
  - Final Output Folder
- **Conversion Settings:**
  - Threads (Numeric Up/Down or Textbox)
  - Resize percentage (e.g., `100%`)
  - Quality (Slider or Textbox, 0-100)
- **Options:**
  - Checkbox: "Delete Source if final size is smaller"
  - Checkbox: "Copy final zip to final folder"
  - Zip Mode: Radio Buttons (Single vs Individual)
- **Execution Controls:** 
  - "Start" button
  - Progress Bar for overall progress.
  - A scrollable Text/Log Box.

### 4. Application Logic implementation
- **Step 1:** Extract archives to subfolders.
- **Step 2:** Delete them when extracted.
- **Step 3:** Replicate folder structure from Source to Target.
- **Step 4:** Launch image conversions via `Parallel.ForEach` using bundled ImageMagick or Magick.NET.
- **Step 5:** Archive the results back to `.cbz`.
- **Step 6:** Check size comparisons and delete source files if chosen.



## Verification Plan

### Manual Verification
- Run the WPF application locally.
- Verify parameters adjust the conversion correctly.
- Ensure commits are correctly pushed to your GitHub repository once the URL is provided.

## AI Instructions

**CRITICAL RULE:** Every time I (the AI) make a change to the codebase, I must automatically make a local `git commit` with a descriptive summary of my edits without needing the user to remind me.
