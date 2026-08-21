using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Text.RegularExpressions;
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
    public List<string> SourceItems { get; set; } = new();
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
    public double TrimMinSize { get; set; } = 0.75;
    public double SmartTrimThreshold { get; set; } = 0.97;
    public double SmartTrimTolerance { get; set; } = 8.0;
    public string ZipMode { get; set; } = string.Empty; // "single" or "individual"
    public bool DeleteTemp { get; set; } = true;
    public bool IncludeRangeInName { get; set; } = true;
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
        
        long initialSize = CalculateInitialSize();
        Report(0, $"Initial size: {initialSize / (1024 * 1024)} MB");

        // We'll work with a local list so we can add extracted folders to it
        var workItems = _ctx.SourceItems.ToList();

        // Step 1: Extract archives
        Report(5, "Step 1 & 2: Extracting archives...");
        workItems = await ExtractArchivesToTempAsync(workItems, cancelToken);

        // Step 3: Create folder structure in Target
        Report(20, "Step 3: Creating target folder structure...");
        ReplicateFolderStructure(workItems, _ctx.TempFolder);

        // Step 4: Convert images to WebP
        Report(25, "Step 4: Converting images to WebP...");
        await ConvertImagesAsync(workItems, cancelToken);

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
                foreach (var item in _ctx.SourceItems)
                {
                    if (Directory.Exists(item)) Directory.Delete(item, true);
                    else if (File.Exists(item)) File.Delete(item);
                }
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

    private long CalculateInitialSize()
    {
        long total = 0;
        foreach (var item in _ctx.SourceItems)
        {
            if (Directory.Exists(item)) total += CalculateDirectorySize(item);
            else if (File.Exists(item)) total += new FileInfo(item).Length;
        }
        return total;
    }

    private async Task<List<string>> ExtractArchivesToTempAsync(List<string> items, CancellationToken cancelToken)
    {
        var exts = new[] { ".zip", ".7z", ".cbz", ".cbr", ".rar" };
        var results = new List<string>();
        string extractionRoot = Path.Combine(_ctx.TempFolder, "_extracted");
        
        bool use7z = !string.IsNullOrWhiteSpace(_ctx.Fallback7z) && File.Exists(_ctx.Fallback7z);

        foreach (var item in items)
        {
            cancelToken.ThrowIfCancellationRequested();

            if (Directory.Exists(item))
            {
                // Scan folder for archives and extract them in-place? 
                // To keep source clean and maintain logic, let's extract files found in folders to Temp too
                var filesInFolder = Directory.GetFiles(item, "*.*", SearchOption.AllDirectories)
                                             .Where(f => exts.Contains(Path.GetExtension(f).ToLowerInvariant()))
                                             .ToList();

                if (filesInFolder.Any())
                {
                    foreach (var file in filesInFolder)
                    {
                        string subDest = Path.Combine(extractionRoot, Path.GetFileNameWithoutExtension(file));
                        await ExtractArchive(file, subDest, use7z, cancelToken);
                        if (_ctx.DeleteSource) File.Delete(file);
                    }
                }
                results.Add(item); // Keep the folder itself
            }
            else if (File.Exists(item))
            {
                string ext = Path.GetExtension(item).ToLowerInvariant();
                if (exts.Contains(ext))
                {
                    string subDest = Path.Combine(extractionRoot, Path.GetFileNameWithoutExtension(item));
                    await ExtractArchive(item, subDest, use7z, cancelToken);
                    results.Add(subDest); // Process the extracted folder instead of the zip
                    // Note: if deleteSource is true, we delete it later or now? Let's do it now for files if requested
                    if (_ctx.DeleteSource) File.Delete(item);
                }
                else
                {
                    results.Add(item); // Not an archive, just a file
                }
            }
        }
        return results;
    }

    private async Task ExtractArchive(string file, string destFolder, bool use7z, CancellationToken cancelToken)
    {
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
                await Task.Run(() => ZipFile.ExtractToDirectory(file, destFolder, true), cancelToken);
            }
            else
            {
                Report(5, $"Cannot extract {file} without 7z.exe configured.");
            }
        }
    }

    private void ReplicateFolderStructure(List<string> items, string targetBase)
    {
        Directory.CreateDirectory(targetBase);
        foreach (var item in items)
        {
            if (Directory.Exists(item))
            {
                string rootName = Path.GetFileName(item);
                string targetRoot = Path.Combine(targetBase, rootName);
                Directory.CreateDirectory(targetRoot);
                foreach (var dirPath in Directory.GetDirectories(item, "*", SearchOption.AllDirectories))
                {
                    string relative = dirPath.Substring(item.Length).TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
                    Directory.CreateDirectory(Path.Combine(targetRoot, relative));
                }
            }
        }
    }

    private async Task ConvertImagesAsync(List<string> items, CancellationToken cancelToken)
    {
        var imageExts = new[] { ".jpg", ".jpeg", ".png", ".bmp", ".webp", ".gif" };
        var tasks = new List<(string SourceFile, string DestFile)>();

        foreach (var item in items)
        {
            if (Directory.Exists(item))
            {
                string rootName = Path.GetFileName(item);
                var files = Directory.GetFiles(item, "*.*", SearchOption.AllDirectories)
                                     .Where(f => imageExts.Contains(Path.GetExtension(f).ToLowerInvariant()));
                foreach (var f in files)
                {
                    string relative = f.Substring(item.Length).TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
                    string dest = Path.Combine(_ctx.TempFolder, rootName, relative);
                    dest = Path.ChangeExtension(dest, ".webp");
                    tasks.Add((f, dest));
                }
            }
            else if (File.Exists(item))
            {
                if (imageExts.Contains(Path.GetExtension(item).ToLowerInvariant()))
                {
                    string dest = Path.Combine(_ctx.TempFolder, Path.GetFileName(item));
                    dest = Path.ChangeExtension(dest, ".webp");
                    tasks.Add((item, dest));
                }
            }
        }

        int total = tasks.Count;
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
            Parallel.ForEach(tasks, parallelOptions, task =>
            {
                try
                {
                    Directory.CreateDirectory(Path.GetDirectoryName(task.DestFile)!);
                    using var image = new MagickImage(task.SourceFile);
                    
                    if (_ctx.TrimPages)
                    {
                        image.ColorFuzz = new Percentage(5);
                        using var testClone = image.Clone() as MagickImage;
                        if (testClone != null)
                        {
                            testClone.Trim();
                            testClone.ResetPage();
                            if (testClone.Width >= image.Width * _ctx.TrimMinSize && testClone.Height >= image.Height * _ctx.TrimMinSize)
                            {
                                image.Trim();
                                image.ResetPage();
                            }
                        }
                        image.ColorFuzz = new Percentage(0);
                    }

                    if (_ctx.SmartTrimPages)
                    {
                        var bounds = CalculateSmartTrimBounds(image, rowBgThreshold: _ctx.SmartTrimThreshold, colorTolerancePct: _ctx.SmartTrimTolerance);
                        if (bounds is not null)
                        {
                            if (bounds.Width >= image.Width * _ctx.TrimMinSize && bounds.Height >= image.Height * _ctx.TrimMinSize)
                            {
                                image.Crop(bounds);
                                image.ResetPage();
                            }
                        }
                    }

                    if (magResize.HasValue) image.Resize(magResize.Value);

                    image.Quality = (uint)_ctx.Quality;
                    image.Format = MagickFormat.WebP;
                    image.Write(task.DestFile);
                }
                catch (Exception ex)
                {
                    if ((ex is DllNotFoundException || ex is TypeInitializationException) && !missingDllLogged)
                    {
                        missingDllLogged = true;
                        Report(0, $"CRITICAL ERROR: Failed to load ImageMagick DLL. Details: {ex.Message}");
                    }
                    // Fallback copy
                    try { File.Copy(task.SourceFile, Path.Combine(Path.GetDirectoryName(task.DestFile)!, Path.GetFileName(task.SourceFile)), true); } catch { }
                }

                int c = Interlocked.Increment(ref completed);
                if (c % 5 == 0 || c == total)
                {
                    int pct = 25 + (int)((c / (double)total) * 55);
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
                var items = new DirectoryInfo(_ctx.TempFolder).GetFileSystemInfos()
                                      .Where(i => i.Name != "_extracted")
                                      .Select(i => i.Name).ToList();
                string baseName = GetSmartBaseName(items);
                string zipPath = GetUniqueFilePath(destDir, $"{baseName}.cbz");
                ZipFile.CreateFromDirectory(_ctx.TempFolder, zipPath, CompressionLevel.NoCompression, false);
                totalSize += new FileInfo(zipPath).Length;
            }
            else if (_ctx.ZipMode == "individual")
            {
                foreach (var dir in Directory.GetDirectories(_ctx.TempFolder))
                {
                    if (Path.GetFileName(dir) == "_extracted") continue;
                    cancelToken.ThrowIfCancellationRequested();
                    var dInfo = new DirectoryInfo(dir);
                    string zipPath = GetUniqueFilePath(destDir, $"{dInfo.Name}.cbz");
                    ZipFile.CreateFromDirectory(dir, zipPath, CompressionLevel.NoCompression, false);
                    totalSize += new FileInfo(zipPath).Length;
                }

                foreach (var file in Directory.GetFiles(_ctx.TempFolder))
                {
                    cancelToken.ThrowIfCancellationRequested();
                    string zipPath = GetUniqueFilePath(destDir, $"{Path.GetFileNameWithoutExtension(file)}.cbz");
                    using var archive = ZipFile.Open(zipPath, ZipArchiveMode.Create);
                    archive.CreateEntryFromFile(file, Path.GetFileName(file), CompressionLevel.NoCompression);
                    totalSize += new FileInfo(zipPath).Length;
                }
            }
            else // "none"
            {
                var items = new DirectoryInfo(_ctx.TempFolder).GetFileSystemInfos()
                                      .Where(i => i.Name != "_extracted")
                                      .Select(i => i.Name).ToList();
                string baseName = GetSmartBaseName(items);
                string targetPath = GetUniqueFolderPath(destDir, baseName);
                CopyDirectory(_ctx.TempFolder, targetPath, "_extracted");
                totalSize += CalculateDirectorySize(targetPath);
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

    private string GetSmartBaseName(IEnumerable<string> items)
    {
        var list = items.OrderBy(x => x).ToList();
        if (list.Count == 0) return new DirectoryInfo(_ctx.TempFolder).Name;

        // Matches common termination markers: v01, vol. 1, ch 10, (2009), 2009
        string markerPattern = @"(?i)[\s\-_.\(\[]*(v(?:ol)?\.?\s*\d+|ch(?:ap)?\.?\s*\d+|\(?\d{4}\)?)";
        var regex = new Regex($"^(.*?){markerPattern}", RegexOptions.IgnoreCase);

        if (list.Count == 1)
        {
            var match = regex.Match(list[0]);
            return match.Success ? match.Groups[1].Value.Trim(' ', '-', '_', '.', '(') : Path.GetFileNameWithoutExtension(list[0]);
        }

        // Multiple items
        var first = list.First();
        var last = list.Last();
        var matchFirst = regex.Match(first);
        var matchLast = regex.Match(last);

        if (matchFirst.Success && matchLast.Success)
        {
            string prefixFirst = matchFirst.Groups[1].Value.Trim(' ', '-', '_', '.', '(');
            string prefixLast = matchLast.Groups[1].Value.Trim(' ', '-', '_', '.', '(');

            if (prefixFirst.Equals(prefixLast, StringComparison.OrdinalIgnoreCase))
            {
                if (!_ctx.IncludeRangeInName) return prefixFirst;

                string markerFirst = matchFirst.Groups[2].Value.Trim(' ', '-', '_', '.', '(', ')');
                string markerLast = matchLast.Groups[2].Value.Trim(' ', '-', '_', '.', '(', ')');
                return $"{prefixFirst} {markerFirst}-{markerLast}";
            }
        }

        // Fallback if no common pattern: use first item basis
        var firstMatch = regex.Match(first);
        return firstMatch.Success ? firstMatch.Groups[1].Value.Trim(' ', '-', '_', '.', '(') : Path.GetFileNameWithoutExtension(first);
    }

    private void CopyDirectory(string sourceDir, string destDir, string? excludeName = null)
    {
        Directory.CreateDirectory(destDir);
        foreach (var file in Directory.GetFiles(sourceDir, "*", SearchOption.AllDirectories))
        {
            string relative = file.Substring(sourceDir.Length).TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            if (!string.IsNullOrEmpty(excludeName) && relative.StartsWith(excludeName)) continue;
            
            string target = Path.Combine(destDir, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            File.Copy(file, target, true);
        }
    }

    private string GetUniqueFolderPath(string destDir, string folderName)
    {
        string fullPath = Path.Combine(destDir, folderName);
        if (!Directory.Exists(fullPath)) return fullPath;

        int i = 1;
        while (Directory.Exists(fullPath = Path.Combine(destDir, $"{folderName}_{i:D3}")))
        {
            i++;
        }
        return fullPath;
    }

    private string GetUniqueFilePath(string destDir, string fileName)
    {
        string fullPath = Path.Combine(destDir, fileName);
        if (!File.Exists(fullPath)) return fullPath;

        string name = Path.GetFileNameWithoutExtension(fileName);
        string ext = Path.GetExtension(fileName);
        int i = 1;
        while (File.Exists(fullPath = Path.Combine(destDir, $"{name}_{i:D3}{ext}")))
        {
            i++;
        }
        return fullPath;
    }
}
