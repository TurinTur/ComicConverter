using System;
using System.IO;
using System.Text;
using System.Threading.Tasks;
using System.Windows;

namespace ComicConverter;

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        TxtTempFolder.Text = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "temp");
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
        bool copyFinal = ChkCopyFinal.IsChecked == true;
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
                CopyFinal = copyFinal,
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