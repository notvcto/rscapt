# add-ffmpeg-to-path.ps1
# Adds ffmpeg\bin to the current user's PATH permanently.
# Run once from the directory where you extracted ffmpeg.
# Usage: right-click → "Run with PowerShell", or:
#   powershell -ExecutionPolicy Bypass -File add-ffmpeg-to-path.ps1

$ffmpegBin = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "ffmpeg\bin"

if (-not (Test-Path $ffmpegBin)) {
    Write-Error "Could not find '$ffmpegBin'. Place this script next to your ffmpeg folder and try again."
    exit 1
}

$current = [Environment]::GetEnvironmentVariable("Path", "User")

if ($current -split ";" | Where-Object { $_ -eq $ffmpegBin }) {
    Write-Host "Already in PATH: $ffmpegBin"
    exit 0
}

[Environment]::SetEnvironmentVariable("Path", "$current;$ffmpegBin", "User")
Write-Host "Added to PATH: $ffmpegBin"
Write-Host "Open a new terminal to use ffmpeg."
