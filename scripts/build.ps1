#Requires -Version 5.1
param(
    [string]$Toolchain = 'stable-x86_64-pc-windows-msvc',
    [string]$LinkerBin
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$product = 'Comet-WebView2'
$dist = Join-Path $projectRoot 'dist'
$outputDir = Join-Path $dist $product
$tempZip = Join-Path $dist ("build-{0}.tmp" -f [guid]::NewGuid())
$savedPath = $env:PATH
$savedLinker = $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER

function Invoke-Cargo {
    & $cargo "+$Toolchain" @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args[0]) failed" }
}

function Add-PackageFile($source, $relativeName) {
    $destination = Join-Path $outputDir $relativeName
    [IO.Directory]::CreateDirectory((Split-Path -Parent $destination)) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
        $package, $destination, "$product/$relativeName") | Out-Null
}

Push-Location $projectRoot
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    Add-Type -AssemblyName System.IO.Compression
    $cargo = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
    if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
    $target = 'x86_64-pc-windows-msvc'
    if ($Toolchain -like '*gnullvm') {
        if (-not $LinkerBin) { throw 'The gnullvm build requires -LinkerBin.' }
        $target = 'x86_64-pc-windows-gnullvm'
        $LinkerBin = (Resolve-Path -LiteralPath $LinkerBin).Path
        $env:PATH = "$LinkerBin;$savedPath"
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = Join-Path $LinkerBin 'x86_64-w64-mingw32-clang.exe'
    }

    Invoke-Cargo build --locked --release --target $target

    # MSVC links the WebView2 loader statically. Only LLVM-MinGW needs its DLL.
    $metadataArgs = if ($target -like '*gnullvm') { '--filter-platform', $target } else { '--no-deps' }
    $metadata = Invoke-Cargo metadata --locked --format-version 1 @metadataArgs | ConvertFrom-Json
    [IO.Directory]::CreateDirectory($dist) | Out-Null
    # Only these build inputs enter the ZIP, even when dist contains older or private files.
    $package = [IO.Compression.ZipFile]::Open($tempZip, [IO.Compression.ZipArchiveMode]::Create)
    try {
        Add-PackageFile (Join-Path $metadata.target_directory "$target/release/comet-webview2.exe") "$product.exe"
        if ($target -like '*gnullvm') {
            $webview = $metadata.packages | Where-Object name -EQ 'webview2-com-sys'
            $loader = Join-Path (Split-Path -Parent $webview.manifest_path) 'x64/WebView2Loader.dll'
            Add-PackageFile $loader 'WebView2Loader.dll'
            Add-PackageFile (Join-Path $LinkerBin 'libunwind.dll') 'libunwind.dll'
            Add-PackageFile (Join-Path (Split-Path -Parent $LinkerBin) 'LICENSE.TXT') 'LLVM-MinGW-LICENSE.txt'
        }
        foreach ($name in 'README.md','assets/comet.png','src/keyboard/keymap.rs','LICENSE','THIRD-PARTY-NOTICES.md','locales/README.md') {
            Add-PackageFile $name $name
        }
        foreach ($locale in Get-ChildItem -LiteralPath locales -Filter '*.json' -File) {
            Add-PackageFile $locale.FullName "locales/$($locale.Name)"
        }
    } finally { $package.Dispose() }
    $archive = Join-Path $dist "$product.zip"
    if ([IO.File]::Exists($archive)) {
        [IO.File]::Replace($tempZip, $archive, [System.Management.Automation.Language.NullString]::Value)
    } else {
        [IO.File]::Move($tempZip, $archive)
    }
    Write-Output "Package: $archive"
} finally {
    if (Test-Path -LiteralPath $tempZip) { Remove-Item -LiteralPath $tempZip }
    $env:PATH = $savedPath
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = $savedLinker
    Pop-Location
}
