using System;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows;

namespace ComicConverter;

public class AppSettings
{
    public string SourceFolder { get; set; } = string.Empty;
    public string TempFolder { get; set; } = string.Empty;
    public string FinalFolder { get; set; } = string.Empty;
    public string Fallback7z { get; set; } = string.Empty;
    public string Threads { get; set; } = string.Empty;
    public string Resize { get; set; } = "100%";
    public string Quality { get; set; } = "67";
    public bool DeleteSource { get; set; } = false;
    public bool DeleteTemp { get; set; } = true;
    public bool CopyFinal { get; set; } = true;
    public bool TrimPages { get; set; } = true;
    public bool SmartTrimPages { get; set; } = false;
    public string TrimMinSize { get; set; } = "75";
    public string SmartTrimThreshold { get; set; } = "97";
    public string SmartTrimTolerance { get; set; } = "8";
    public string ZipMode { get; set; } = "single";
}

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        TxtTempFolder.Text = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "temp");
        TxtThreads.Text = (Environment.ProcessorCount -1).ToString();
        LoadSettings();
    }

    private readonly string _settingsFile = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "settings.json");

    private void LoadSettings()
    {
        try
        {
            if (File.Exists(_settingsFile))
            {
                var json = File.ReadAllText(_settingsFile);
                var settings = JsonSerializer.Deserialize<AppSettings>(json);
                if (settings != null)
                {
                    TxtSourceFolder.Text = settings.SourceFolder;
                    if (!string.IsNullOrWhiteSpace(settings.TempFolder)) TxtTempFolder.Text = settings.TempFolder;
                    TxtFinalFolder.Text = settings.FinalFolder;
                    Txt7zPath.Text = settings.Fallback7z;
                    if (!string.IsNullOrWhiteSpace(settings.Threads)) TxtThreads.Text = settings.Threads;
                    TxtResize.Text = settings.Resize;
                    TxtQuality.Text = settings.Quality;
                    ChkDeleteSource.IsChecked = settings.DeleteSource;
                    ChkDeleteTemp.IsChecked = settings.DeleteTemp;
                    ChkCopyFinal.IsChecked = settings.CopyFinal;
                    ChkTrimPages.IsChecked = settings.TrimPages;
                    ChkSmartTrimPages.IsChecked = settings.SmartTrimPages;
                    TxtTrimMinSize.Text = settings.TrimMinSize;
                    TxtSmartTrimThreshold.Text = settings.SmartTrimThreshold;
                    TxtSmartTrimTolerance.Text = settings.SmartTrimTolerance;
                    RbZipSingle.IsChecked = settings.ZipMode == "single";
                    RbZipIndividual.IsChecked = settings.ZipMode == "individual";
                }
            }
        }
        catch { /* ignored */ }
    }

    private void SaveSettings()
    {
        try
        {
            var settings = new AppSettings
            {
                SourceFolder = TxtSourceFolder.Text,
                TempFolder = TxtTempFolder.Text,
                FinalFolder = TxtFinalFolder.Text,
                Fallback7z = Txt7zPath.Text,
                Threads = TxtThreads.Text,
                Resize = TxtResize.Text,
                Quality = TxtQuality.Text,
                DeleteSource = ChkDeleteSource.IsChecked == true,
                DeleteTemp = ChkDeleteTemp.IsChecked == true,
                CopyFinal = ChkCopyFinal.IsChecked == true,
                TrimPages = ChkTrimPages.IsChecked == true,
                SmartTrimPages = ChkSmartTrimPages.IsChecked == true,
                TrimMinSize = TxtTrimMinSize.Text,
                SmartTrimThreshold = TxtSmartTrimThreshold.Text,
                SmartTrimTolerance = TxtSmartTrimTolerance.Text,
                ZipMode = RbZipSingle.IsChecked == true ? "single" : "individual"
            };
            var json = JsonSerializer.Serialize(settings, new JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText(_settingsFile, json);
        }
        catch { /* ignored */ }
    }

    private void Window_Closing(object sender, System.ComponentModel.CancelEventArgs e)
    {
        SaveSettings();
    }

    private void LogActivity(string message)
    {
        Dispatcher.Invoke(() =>
        {
            TxtLog.AppendText($"[{DateTime.Now:HH:mm:ss}] {message}\n");
            TxtLog.ScrollToEnd();
        });
    }

    private void BtnBrowseSource_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog { Title = "Select Source Folder" };
        if (dialog.ShowDialog() == true)
        {
            TxtSourceFolder.Text = dialog.FolderName;
        }
    }

    private void BtnBrowseTemp_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog { Title = "Select Temp/Target Folder" };
        if (dialog.ShowDialog() == true)
        {
            TxtTempFolder.Text = dialog.FolderName;
        }
    }

    private void BtnBrowseFinal_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog { Title = "Select Final Output Folder" };
        if (dialog.ShowDialog() == true)
        {
            TxtFinalFolder.Text = dialog.FolderName;
        }
    }

    private void BtnBrowse7z_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Title = "Select 7z.exe",
            Filter = "Executables (*.exe)|*.exe|All files (*.*)|*.*",
            FileName = "7z.exe"
        };
        if (dialog.ShowDialog() == true)
        {
            Txt7zPath.Text = dialog.FileName;
        }
    }

    private async void BtnStart_Click(object sender, RoutedEventArgs e)
    {
        // Basic validation
        if (string.IsNullOrWhiteSpace(TxtSourceFolder.Text) ||
            string.IsNullOrWhiteSpace(TxtTempFolder.Text) ||
            string.IsNullOrWhiteSpace(TxtFinalFolder.Text))
        {
            MessageBox.Show("Please select Source, Temp, and Final folders.", "Missing Paths", MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }

        BtnStart.IsEnabled = false;
        PbOverall.Value = 0;
        TxtLog.Clear();
        TxtStatus.Text = "Processing...";
        LogActivity("Process started...");

        // Extract settings
        string source = TxtSourceFolder.Text;
        string temp = TxtTempFolder.Text;
        string final = TxtFinalFolder.Text;
        string fallback7z = Txt7zPath.Text;
        int.TryParse(TxtThreads.Text, out int threads);
        if (threads <= 0) threads = Environment.ProcessorCount;
        
        string resize = TxtResize.Text;
        int.TryParse(TxtQuality.Text, out int quality);
        if (quality <= 0) quality = 67;

        bool deleteSource = ChkDeleteSource.IsChecked == true;
        bool deleteTemp = ChkDeleteTemp.IsChecked == true;
        bool copyFinal = ChkCopyFinal.IsChecked == true;
        bool trimPages = ChkTrimPages.IsChecked == true;
        bool smartTrimPages = ChkSmartTrimPages.IsChecked == true;
        
        double.TryParse(TxtTrimMinSize.Text, out double trimMinSizePct);
        if (trimMinSizePct <= 0) trimMinSizePct = 75;
        double trimMinSize = trimMinSizePct / 100.0;

        double.TryParse(TxtSmartTrimThreshold.Text, out double smartTrimThresholdPct);
        if (smartTrimThresholdPct <= 0) smartTrimThresholdPct = 97;
        double smartTrimThreshold = smartTrimThresholdPct / 100.0;

        double.TryParse(TxtSmartTrimTolerance.Text, out double smartTrimTolerance);
        if (smartTrimTolerance < 0) smartTrimTolerance = 8.0;

        string zipMode = RbZipSingle.IsChecked == true ? "single" : "individual";

        try
        {
            var ctx = new ProcessorContext
            {
                SourceFolder = source,
                TempFolder = temp,
                FinalFolder = final,
                Fallback7z = fallback7z,
                Threads = threads,
                Resize = resize,
                Quality = quality,
                DeleteSource = deleteSource,
                DeleteTemp = deleteTemp,
                CopyFinal = copyFinal,
                TrimPages = trimPages,
                SmartTrimPages = smartTrimPages,
                TrimMinSize = trimMinSize,
                SmartTrimThreshold = smartTrimThreshold,
                SmartTrimTolerance = smartTrimTolerance,
                ZipMode = zipMode
            };

            var progress = new Progress<ProgressReport>(report =>
            {
                PbOverall.Value = report.Percentage;
                LogActivity(report.Message);
            });

            var processor = new ComicProcessor(ctx, progress);
            
            await processor.ProcessAsync(CancellationToken.None);

            TxtStatus.Text = "Completed";
        }
        catch (Exception ex)
        {
            LogActivity($"ERROR: {ex.Message}");
            TxtStatus.Text = "Error";
        }
        finally
        {
            BtnStart.IsEnabled = true;
        }
    }
}