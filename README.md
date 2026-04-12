# ComicConverter

A WPF application for converting comic archives to WebP format.

## Release Notes

**IMPORTANT:** When dropping a new release archive (ZIP) for GitHub, ensure that you include the ImageMagick/Magick.NET DLL. If you forget to include the DLL, the program will open without errors, but the image conversion to WebP will quietly fail or throw critical errors!

1. Compile the app in Release mode.
2. Package the contents of the `bin/Release/...` directory.
3. Make sure that `Magick.NET` DLLs (like `Magick.Native-Q16-AnyCPU.dll` depending on your version and platform) are in the ZIP folder along with `ComicConverter.exe`!

## Publishing the Production Binary

To create a single-file production binary or a standalone package, you can use the `dotnet publish` command. Open your terminal in the project directory and run:

```bash
dotnet publish -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true
dotnet publish -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:DebugType=None -o C:\temp\ComicConverter\publish
dotnet publish -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:DebugType=None -o C:\temp\ComicConverter\publish
dotnet publish -c Release -r win-x64 --self-contained false -p:PublishSingleFile=true -p:DebugType=None -o C:\temp\ComicConverter\publish_small
```
gh release create v1.0.2 "C:\temp\ComicConverter\publish\Comic Webp Conveter 1.02.zip" --title "Comic Converter v1.0.2" --notes "Mutiple file selection added"

Take the output from `bin\Release\net8.0-windows\win-x64\publish` and package it.

**IMPORTANT:** Always remember to commit and push your changes to GitHub before publishing a new release!
```ps1
git add .
git commit -m "Your release message"
git push origin master
```
(Repository URL: `https://github.com/TurinTur/ComicConverter.git`)

## Creating a GitHub Release

Once you have pushed your code and created the ZIP file containing the production binary and DLLs, you can create a release on GitHub:

1. Go to your repository on GitHub: `https://github.com/TurinTur/ComicConverter`
2. On the right side, click on **Releases**, then click **Draft a new release**.
3. Click **Choose a tag** and type a version number (e.g., `v1.0.0`), then click **Create new tag**.
4. Set the **Release title** (e.g., `ComicConverter v1.0.0`).
5. Write a description of what changed in this version.
6. Drag and drop your final ZIP file (containing `ComicConverter.exe`, `.dll`s, etc.) into the **Attach binaries by dropping them here or selecting them** box at the bottom.
7. Click **Publish release**.
