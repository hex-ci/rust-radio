# 打包 Windows 发布包：rust-radio.exe + 许可文本 + 图标，输出 dist\rust-radio_<版本>_windows_<架构>.zip
# 用法：pwsh scripts\package-windows.ps1            # 版本从 Cargo.toml 读
#       $env:APP_VERSION = "0.2.0"; pwsh scripts\package-windows.ps1
# 需要先 cargo build --release -p rust-radio-app。
param(
    [string]$Version = $env:APP_VERSION
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $cargo = Get-Content (Join-Path $root "Cargo.toml") -Raw
    $m = [regex]::Match($cargo, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
    if (-not $m.Success) { throw "无法从 Cargo.toml 读取 workspace 版本" }
    $Version = $m.Groups[1].Value
}

$arch = if ($env:APP_ARCH) { $env:APP_ARCH } else { "x86_64" }
$name = "rust-radio_${Version}_windows_${arch}"
$stage = Join-Path $root "dist\$name"
$zip = Join-Path $root "dist\$name.zip"

$exe = Join-Path $root "target\release\rust-radio.exe"
if (-not (Test-Path $exe)) { throw "缺少 target\release\rust-radio.exe，先 cargo build --release -p rust-radio-app" }

Write-Host "==> 组装 $stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item $exe (Join-Path $stage "rust-radio.exe")
Copy-Item README.md, LICENSE $stage
Copy-Item "crates\rust-radio-app\assets\app-icon-1024.png" (Join-Path $stage "rust-radio.png")

Write-Host "==> 压缩 $zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal

Write-Host "==> 完成：$zip"
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::OpenRead($zip).Entries | ForEach-Object { "  {0}  {1:N0} B" -f $_.FullName, $_.Length }