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
    public bool SmartTrimPages { get; set; }
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
                        // Use a fuzz factor so near-white (slightly off-white) borders are also trimmed
                        image.ColorFuzz = new Percentage(5);
                        
                        using var testClone = image.Clone() as MagickImage;
                        if (testClone != null)
                        {
                            testClone.Trim();
                            testClone.ResetPage();
                            
                            // trim:minSize=75% means the trimmed result must be >= 75% of original
                            // in EACH dimension (width and height), matching magick's behaviour
                            bool widthOk  = testClone.Width  >= image.Width  * 0.75;
                            bool heightOk = testClone.Height >= image.Height * 0.75;
                            
                            if (widthOk && heightOk)
                            {
                                image.Trim();
                                image.ResetPage();
                            }
                            // else: skip trim — border was too large relative to the image
                        }
                        
                        image.ColorFuzz = new Percentage(0); // reset fuzz
                    }

                    // Smart Trim: trim borders where >= 97% of pixels are background,
                    // even if a small portion (like a page number) interrupts the edge
                    if (_ctx.SmartTrimPages)
                    {
                        var bounds = CalculateSmartTrimBounds(image, rowBgThreshold: 0.97, colorTolerancePct: 8.0);
                        if (bounds is not null)
                        {
                            bool widthOk  = bounds.Width  >= image.Width  * 0.75;
                            bool heightOk = bounds.Height >= image.Height * 0.75;
                            if (widthOk && heightOk)
                            {
                                image.Crop(bounds);
                                image.ResetPage();
                            }
                        }
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

    /// <summary>
    /// Scans edges row-by-row / column-by-column and returns a crop geometry that
    /// strips any edge strip where >= rowBgThreshold fraction of pixels are within
    /// colorTolerancePct of the detected background colour.
    /// This allows trimming borders that contain small elements (e.g. page numbers).
    /// </summary>
    private static MagickGeometry? CalculateSmartTrimBounds(
        MagickImage image,
        double rowBgThreshold  = 0.97,
        double colorTolerancePct = 8.0)
    {
        int width  = (int)image.Width;
        int height = (int)image.Height;
        if (width <= 1 || height <= 1) return null;

        var px = image.GetPixels();

        // Average the 4 corner pixels to estimate background colour
        double bgR = 0, bgG = 0, bgB = 0;
        int cornerCount = 0;
        foreach (var (cx, cy) in new[] { (0, 0), (width-1, 0), (0, height-1), (width-1, height-1) })
        {
            var c = px.GetPixel(cx, cy)?.ToColor();
            if (c != null) { bgR += c.R; bgG += c.G; bgB += c.B; cornerCount++; }
        }
        if (cornerCount == 0) return null;
        bgR /= cornerCount; bgG /= cornerCount; bgB /= cornerCount;

        // tolerance: average per-channel deviation allowed to be considered "background"
        double tolerance = Quantum.Max * (colorTolerancePct / 100.0);

        bool IsBackground(IPixel<ushort>? p)
        {
            if (p == null) return true;
            var c = p.ToColor();
            if (c == null) return true;
            return (Math.Abs((double)c.R - bgR)
                  + Math.Abs((double)c.G - bgG)
                  + Math.Abs((double)c.B - bgB)) / 3.0 <= tolerance;
        }

        double RowBgFraction(int y, int xFrom, int xTo)
        {
            int count = 0, total = xTo - xFrom + 1;
            for (int x = xFrom; x <= xTo; x++)
                if (IsBackground(px.GetPixel(x, y))) count++;
            return total > 0 ? (double)count / total : 1.0;
        }

        double ColBgFraction(int x, int yFrom, int yTo)
        {
            int count = 0, total = yTo - yFrom + 1;
            for (int y = yFrom; y <= yTo; y++)
                if (IsBackground(px.GetPixel(x, y))) count++;
            return total > 0 ? (double)count / total : 1.0;
        }

        // Scan from each edge inward while the strip is >= threshold background
        int top = 0;
        for (int y = 0; y < height / 2; y++)
        {
            if (RowBgFraction(y, 0, width - 1) >= rowBgThreshold) top = y + 1;
            else break;
        }

        int bottom = height - 1;
        for (int y = height - 1; y > top; y--)
        {
            if (RowBgFraction(y, 0, width - 1) >= rowBgThreshold) bottom = y - 1;
            else break;
        }

        int left = 0;
        for (int x = 0; x < width / 2; x++)
        {
            if (ColBgFraction(x, top, bottom) >= rowBgThreshold) left = x + 1;
            else break;
        }

        int right = width - 1;
        for (int x = width - 1; x > left; x--)
        {
            if (ColBgFraction(x, top, bottom) >= rowBgThreshold) right = x - 1;
            else break;
        }

        if (left >= right || top >= bottom) return null;
        // No meaningful trim found
        if (left == 0 && top == 0 && right == width - 1 && bottom == height - 1) return null;

        return new MagickGeometry(
            left, top,
            (uint)(right - left + 1),
            (uint)(bottom - top + 1));
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
