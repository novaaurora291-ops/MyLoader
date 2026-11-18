# End-to-end engine test on Windows: generates a test clip, applies the bundled
# Alight Motion HDR preset, renders a still and exports 1080p60 + 4K60 MP4s
# with the best hardware encoder. Run from the repo root:
#   powershell -ExecutionPolicy Bypass -File scripts\smoke-test.ps1

$ErrorActionPreference = "Stop"
$out = "test-output"
New-Item -ItemType Directory -Force $out | Out-Null

Write-Host "==> Building engine CLI (release)" -ForegroundColor Cyan
cargo build --release -p myloader-engine
$cli = "target\release\mlcli.exe"

Write-Host "==> Engine info (GPU + tested encoders)" -ForegroundColor Cyan
& $cli info

Write-Host "==> Generating 10 s 1080p60 test clip with audio" -ForegroundColor Cyan
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc2=s=1920x1080:r=60:d=10" -f lavfi -i "sine=f=440:d=10" `
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest "$out\input.mp4"

Write-Host "==> Parsing the bundled Alight Motion preset" -ForegroundColor Cyan
& $cli parse presets\hdr-cc-lyrics.xml

Write-Host "==> Creating a 9:16 project (video + preset, filled to composition)" -ForegroundColor Cyan
& $cli new "$out\input.mp4" -o "$out\project.mlproj" --aspect 9:16 --preset presets\hdr-cc-lyrics.xml

Write-Host "==> Rendering a still" -ForegroundColor Cyan
& $cli still "$out\project.mlproj" -o "$out\still.png" --t 2 --res 1080

Write-Host "==> GPU benchmark (4K)" -ForegroundColor Cyan
& $cli bench --res 2160 --frames 120

Write-Host "==> Export 1080p 60 fps" -ForegroundColor Cyan
& $cli export "$out\project.mlproj" -o "$out\export_1080p60.mp4" --res 1080 --fps 60

Write-Host "==> Export 4K 60 fps" -ForegroundColor Cyan
& $cli export "$out\project.mlproj" -o "$out\export_4k60.mp4" --res 2160 --fps 60

ffprobe -v error -show_entries stream=codec_name,width,height,r_frame_rate -of compact "$out\export_4k60.mp4"
Write-Host "All good. Outputs are in $out\" -ForegroundColor Green
