using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using ImageMagick;

namespace ComicConverter;

public class ProgressReport
{
    public int Percentage { get; set; }
    public string Message { get; set; } = string.Empty;
}

public class ProcessorContext
{
    public string SourceFolder { get; set; } = string.Empty;
    public string TempFolder { get; set; } = string.Empty;
    public string FinalFolder { get; set; } = string.Empty;
    public string Fallback7z { get; set; } = string.Empty;
    public int Threads { get; set; }
    public string Resize { get; set; } = string.Empty;
    public int Quality { get; set; }
    public bool DeleteSource { get; set; }
    public bool CopyFinal { get; set; }
    public bool TrimPages { get; set; }
    public string ZipMode { get; set; } = string.Empty; // "single" or "individual"
    public bool DeleteTemp { get; set; } = true;
}

public class ComicProcessor
{
    private readonly ProcessorContext _ctx;
    private readonly IProgress<ProgressReport> _progress;

    public ComicProcessor(ProcessorContext ctx, IProgress<ProgressReport> progress)
    {
        _ctx = ctx;
        _progress = progress;
    }

    private void Report(int pct, string msg)
    {
        _progress?.Report(new ProgressReport { Percentage = pct, Message = msg });
    }

    public async Task ProcessAsync(CancellationToken cancelToken)
    {
        var stopwatch = Stopwatch.StartNew();
        
        long initialSize = CalculateDirectorySize(_ctx.SourceFolder);
        Report(0, $"Initial size: {initialSize / (1024 * 1024)} MB");

        // Step 1: Extract archives
        Report(5, "Step 1 & 2: Extracting archives...");
        await ExtractArchivesAsync(cancelToken);

        // Step 3: Create folder structure in Target
        Report(20, "Step 3: Creating target folder structure...");
        ReplicateFolderStructure(_ctx.SourceFolder, _ctx.TempFolder);

        // Step 4: Convert images to WebP
        Report(25, "Step 4: Converting images to WebP...");
        await ConvertImagesAsync(cancelToken);

        long finalSize = CalculateDirectorySize(_ctx.TempFolder);
        Report(80, $"Conversion complete. Target temp size: {finalSize / (1024 * 1024)} MB");

        // Step 5: Archive results
        Report(85, "Step 5: Archiving results to CBZ...");
        string destDir = _ctx.CopyFinal ? _ctx.FinalFolder : _ctx.TempFolder;
        Directory.CreateDirectory(destDir);
        long finalArchiveSize = await ArchiveResultsAsync(destDir, cancelToken);

        // Calculate final zip size
        string sizeReport = $"Final converted size: {finalArchiveSize / (1024 * 1024)} MB";
        if (initialSize > 0 && finalArchiveSize < initialSize)
        {
            double reduction = ((double)(initialSize - finalArchiveSize) / initialSize) * 100;
            sizeReport += $", {Math.Round(reduction)}% size reduction improvement";
        }
        else if (initialSize > 0 && finalArchiveSize > initialSize)
        {
            double increase = ((double)(finalArchiveSize - initialSize) / initialSize) * 100;
            sizeReport += $", {Math.Round(increase)}% size increase";
        }
        Report(95, sizeReport);

        // Step 6: Cleanup Source if smaller
        if (_ctx.DeleteSource)
        {
            if (finalArchiveSize <= initialSize && finalArchiveSize > 0)
            {
                Report(98, "Step 6: Final size is smaller. Deleting source files...");
                DeleteDirectoryContents(_ctx.SourceFolder);
            }
            else
            {
                Report(98, "Step 6: Final size is NOT smaller. Keeping source files.");
            }
        }

        // Cleanup Temp Folder
        if (_ctx.DeleteTemp)
        {
            Report(99, "Cleaning up temp folder...");
            DeleteDirectoryContents(_ctx.TempFolder);
        }
        else
        {
            Report(99, "Skipping temp folder cleanup.");
        }

        stopwatch.Stop();
        Report(100, $"Process completed in {stopwatch.Elapsed.TotalMinutes:F2} minutes.");
    }

    private async Task ExtractArchivesAsync(CancellationToken cancelToken)
    {
        var exts = new[] { ".zip", ".7z", ".cbz", ".cbr", ".rar" };
        var filesToExtract = Directory.GetFiles(_ctx.SourceFolder, "*.*", SearchOption.AllDirectories)
                                      .Where(f => exts.Contains(Path.GetExtension(f).ToLowerInvariant()))
                                      .ToList();

        if (!filesToExtract.Any()) return;

        bool use7z = !string.IsNullOrWhiteSpace(_ctx.Fallback7z) && File.Exists(_ctx.Fallback7z);

        foreach (var file in filesToExtract)
        {
            cancelToken.ThrowIfCancellationRequested();
            string destFolder = Path.Combine(_ctx.SourceFolder, Path.GetFileNameWithoutExtension(file));
            Directory.CreateDirectory(destFolder);

            Report(5, $"Extracting: {Path.GetFileName(file)}");

            if (use7z)
            {
                var pInfo = new ProcessStartInfo
                {
                    FileName = _ctx.Fallback7z,
                    Arguments = $"x \"{file}\" -o\"{destFolder}\" -y",
                    UseShellExecute = false,
                    CreateNoWindow = true
                };
                using var p = Process.Start(pInfo);
                if (p != null) await p.WaitForExitAsync(cancelToken);
            }
            else
            {
                string ext = Path.GetExtension(file).ToLowerInvariant();
                if (ext == ".zip" || ext == ".cbz")
                {
                    await Task.Run(() =>
                    {
                        ZipFile.ExtractToDirectory(file, destFolder, true);
                    }, cancelToken);
                }
                else
                {
                    Report(5, $"Cannot extract {file} without 7z.exe configured.");
                    continue;
                }
            }

            // Step 2: Delete archive after extraction
            File.Delete(file);
        }
    }

    private void ReplicateFolderStructure(string source, string target)
    {
        Directory.CreateDirectory(target);
        foreach (var dirPath in Directory.GetDirectories(source, "*", SearchOption.AllDirectories))
        {
            string newDir = dirPath.Replace(source, target);
            Directory.CreateDirectory(newDir);
        }
    }

    private async Task ConvertImagesAsync(CancellationToken cancelToken)
    {
        var allFiles = Directory.GetFiles(_ctx.SourceFolder, "*.*", SearchOption.AllDirectories).ToList();
        var imageExts = new[] { ".jpg", ".jpeg", ".png", ".bmp", ".webp", ".gif" };
        
        var filesToProcess = allFiles.Where(f => imageExts.Contains(Path.GetExtension(f).ToLowerInvariant())).ToList();
        int total = filesToProcess.Count;
        int completed = 0;
        bool missingDllLogged = false;

        if (total == 0) return;

        var parallelOptions = new ParallelOptions
        {
            MaxDegreeOfParallelism = _ctx.Threads,
            CancellationToken = cancelToken
        };

        // Parse resize
        Percentage? magResize = null;
        if (!string.IsNullOrWhiteSpace(_ctx.Resize) && _ctx.Resize != "100%")
        {
            if (_ctx.Resize.EndsWith("%") && double.TryParse(_ctx.Resize.TrimEnd('%'), out double p))
            {
                magResize = new Percentage(p);
            }
        }

        await Task.Run(() =>
        {
            Parallel.ForEach(filesToProcess, parallelOptions, file =>
            {
                try
                {
                    string relativePath = file.Substring(_ctx.SourceFolder.Length).TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
                    string destPath = Path.Combine(_ctx.TempFolder, relativePath);
                    destPath = Path.ChangeExtension(destPath, ".webp");
                    
                    using var image = new MagickImage(file);
                    
                    if (_ctx.TrimPages)
                    {
                        var originalArea = image.Width * image.Height;
                        image.Trim();
                        var newArea = image.Width * image.Height;
                        
                        // Emulate trim:minSize=75%
                        if (newArea < originalArea * 0.75)
                        {
                            // Reset image if trimmed too much, by rolling back.
                            // Magick doesn't have a simple undo for Trim, so we just reload.
                            image.Read(file);
                        }
                    }
                    else
                    {
                        // Repage not strictly needed for WebP out, Trim does the job
                    }

                    if (magResize.HasValue)
                    {
                        image.Resize(magResize.Value);
                    }

                    image.Quality = (uint)_ctx.Quality;
                    image.Format = MagickFormat.WebP;
                    image.Write(destPath);
                }
                catch (Exception ex)
                {
                    if ((ex is DllNotFoundException || ex is TypeInitializationException) && !missingDllLogged)
                    {
                        missingDllLogged = true;
                        Report(0, $"CRITICAL ERROR: Failed to load ImageMagick DLL. Ensure the Magick.NET DLL is present. Details: {ex.Message}");
                    }
                    else if (!missingDllLogged)
                    {
                        Debug.WriteLine($"Failed to process {file}: {ex.Message}");
                    }

                    // Copy original file as fallback if conversion fails
                    string relativePath = file.Substring(_ctx.SourceFolder.Length).TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
                    string destPath = Path.Combine(_ctx.TempFolder, relativePath);
                    File.Copy(file, destPath, true);
                }

                int c = Interlocked.Increment(ref completed);
                if (c % 5 == 0 || c == total)
                {
                    int pct = 25 + (int)((c / (double)total) * 55); // runs from 25% to 80%
                    Report(pct, $"Converted {c} of {total} files");
                }
            });
        });
    }

    private async Task<long> ArchiveResultsAsync(string destDir, CancellationToken cancelToken)
    {
        return await Task.Run(() =>
        {
            long totalSize = 0;
            if (_ctx.ZipMode == "single")
            {
                // Single CBZ for all items
                var firstItem = new DirectoryInfo(_ctx.TempFolder).GetFileSystemInfos().FirstOrDefault();
                string baseName = firstItem != null ? Path.GetFileNameWithoutExtension(firstItem.Name) : new DirectoryInfo(_ctx.TempFolder).Name;
                string zipPath = Path.Combine(destDir, $"{baseName}.cbz");

                if (File.Exists(zipPath)) File.Delete(zipPath);
                ZipFile.CreateFromDirectory(_ctx.TempFolder, zipPath, CompressionLevel.NoCompression, false);
                totalSize += new FileInfo(zipPath).Length;
            }
            else // "individual"
            {
                foreach (var dir in Directory.GetDirectories(_ctx.TempFolder))
                {
                    cancelToken.ThrowIfCancellationRequested();
                    var dInfo = new DirectoryInfo(dir);
                    string zipPath = Path.Combine(destDir, $"{dInfo.Name}.cbz");
                    if (File.Exists(zipPath)) File.Delete(zipPath);
                    ZipFile.CreateFromDirectory(dir, zipPath, CompressionLevel.NoCompression, false);
                    totalSize += new FileInfo(zipPath).Length;
                }

                foreach (var file in Directory.GetFiles(_ctx.TempFolder))
                {
                    cancelToken.ThrowIfCancellationRequested();
                    string zipPath = Path.Combine(destDir, $"{Path.GetFileNameWithoutExtension(file)}.cbz");
                    if (File.Exists(zipPath)) File.Delete(zipPath);
                    using var archive = ZipFile.Open(zipPath, ZipArchiveMode.Create);
                    archive.CreateEntryFromFile(file, Path.GetFileName(file), CompressionLevel.NoCompression);
                    totalSize += new FileInfo(zipPath).Length;
                }
            }
            return totalSize;
        }, cancelToken);
    }

    private long CalculateDirectorySize(string directory)
    {
        if (!Directory.Exists(directory)) return 0;
        return new DirectoryInfo(directory).EnumerateFiles("*.*", SearchOption.AllDirectories).Sum(f => f.Length);
    }

    private void DeleteDirectoryContents(string directory)
    {
        if (!Directory.Exists(directory)) return;
        var di = new DirectoryInfo(directory);

        foreach (FileInfo file in di.GetFiles())
        {
            file.Delete();
        }
        foreach (DirectoryInfo dir in di.GetDirectories())
        {
            dir.Delete(true);
        }
    }
}
