# MyLoader - one-shot dependency installer for Windows 10/11 (run in PowerShell)
#   Set-ExecutionPolicy -Scope Process Bypass; .\scripts\setup-windows.ps1
# Installs: Visual Studio 2022 Build Tools (C++), Rust (rustup), Node.js LTS, FFmpeg (full, with NVENC/AMF/QSV), Git, WebView2.

$ErrorActionPreference = "Stop"

function Install-Winget($id, $extra = @()) {
    Write-Host "==> $id" -ForegroundColor Cyan
    winget install --id $id -e --accept-source-agreements --accept-package-agreements @extra
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189) { Write-Warning "$id returned $LASTEXITCODE (may already be installed)" }
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    throw "winget not found. Install 'App Installer' from the Microsoft Store first."
}

Install-Winget "Git.Git"
Install-Winget "Microsoft.VisualStudio.2022.BuildTools" @("--override", "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended")
Install-Winget "Rustlang.Rustup"
Install-Winget "OpenJS.NodeJS.LTS"
Install-Winget "Gyan.FFmpeg"
Install-Winget "Microsoft.EdgeWebView2Runtime"

# Refresh PATH for this session
$env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

rustup default stable-msvc
rustup update

Write-Host ""
Write-Host "Versions:" -ForegroundColor Green
rustc --version
cargo --version
node --version
npm --version
ffmpeg -hide_banner -version | Select-Object -First 1
Write-Host ""
Write-Host "Done. Open a NEW terminal, then: npm install ; npm run tauri dev" -ForegroundColor Green
