# MyLoader — GPU Colour Grading & Compositing Engine (Windows)

Desktop video colour-grading / compositing engine that works like **Alight Motion XML presets (HDR CC workflows)** — but on a PC, on the GPU, with hardware-accelerated export.

| | |
|---|---|
| **Core engine** | Rust · **wgpu** (DirectX 12 / Vulkan) WGSL shaders · FFmpeg (decode + NVENC / AMF / QSV encode) |
| **UI** | **Tauri 2** + React 19 + TypeScript · dark timeline UI · real-time **WebGL2** GPU preview |
| **Presets** | Alight Motion `.xml` parser (layers, rectangles, blend modes, opacity, effects) |
| **Export** | 1080p / 1440p / **4K**, up to **60 fps** MP4 (H.264 / HEVC), audio passthrough |

---

## Features

- **Alight Motion `.xml` parser** — reads `<scene>` size/fps/duration, every `<shape>` layer (z-order), `blending` (Multiply, Screen, Overlay, Add, Soft/Hard Light, Darken, Lighten, Dodge, Burn, Difference), opacity (static or first keyframe), start/end time, size × scale transforms, fill colours (`#AARRGGBB`), and effects.
  - "Copy Background" (`effects.lift`, fill = 0) shapes become **adjustment layers**.
  - Mapped effects: `brightcont`/`brightcont2`, `coloradj`, `exposure`, `highlightshadow`, `gaussianblur`/box/lens blur, `vignette`, `satvib`, plus any effect exposing `brightness`/`contrast`/`exposure`/`saturation` params.
  - Transparent→black vertical gradient shapes become a **bottom black fade** (lyrics).
  - Solid shapes become **rectangle layers** (circles get a full corner radius). Unsupported effects are kept as metadata and listed in the inspector — nothing is silently dropped.
- **Multi-layer timeline & adjustment stack** — raw video on the bottom track, any number of CC adjustment / rectangle layers above. Per layer: 👁 hide/unhide, 🔒 lock/unlock, drag-to-reorder (⠿ handle) or ▲▼, drag clips in time, delete, undo/redo.
- **Aspect ratio buttons** — 9:16 (1080×1920), 16:9 (1920×1080), 1:1 (1080×1080), 4:5.
- **⤢ Fill All Layers to Composition** — one click fits the video (Fill/crop, Fit/bars or Stretch) and stretches every unlocked adjustment/rectangle layer to the canvas.
- **Real-time GPU CC sliders** — Exposure, Brightness, Contrast, **Highlights / Shadows (HDR tone)**, Saturation, **Gaussian Blur**, **Vignette** (+softness), **Bottom Black Fade** (+height). Double-click a slider to reset. Each adjustment layer also has opacity + blend mode (e.g. Screen-blended blur = bloom).
- **Fast GPU export** — parallel pipeline: FFmpeg hardware decode → wgpu render (16-bit float HDR intermediates) → NVENC / AMF / QSV encode. Encoders are *test-encoded* at startup so only ones that really work on your PC are offered; Auto picks the fastest. CPU fallbacks for both rendering and encoding.

The preview (WebGL2) and export (wgpu/WGSL) run **the same shader maths**, so what you see is what you export.

---

## Folder structure

```
MyLoader/
├─ crates/engine/                 Rust engine (library + `mlcli` CLI)
│  └─ src/
│     ├─ alight.rs                Alight Motion XML parser / importer
│     ├─ model.rs                 Project / Layer / CC model (JSON, shared with UI)
│     ├─ layout.rs                Aspect presets + Fill All Layers to Composition
│     ├─ render.rs                Frame plan (passes + shader params)
│     ├─ gpu/mod.rs, composite.wgsl   wgpu renderer + WGSL shaders
│     ├─ cpu.rs                   CPU reference renderer (fallback + tests)
│     ├─ ffmpeg.rs                Probe, encoder detection, decode/encode pipes
│     ├─ export.rs                Threaded decode → render → encode pipeline
│     └─ bin/mlcli.rs             Command-line front-end
├─ src-tauri/                     Tauri 2 desktop shell (commands, config, icons)
├─ src/                           React + TypeScript UI
│  ├─ components/                 Viewer, Timeline, Inspector, ExportDialog, Slider
│  ├─ engine/                     WebGL2 preview renderer (shaders.ts = WGSL port)
│  └─ lib/                        store (undo/redo), layout, Tauri API wrapper
├─ presets/hdr-cc-lyrics.xml      Sample Alight Motion HDR CC + lyrics preset
├─ scripts/setup-windows.ps1      Installs all dependencies with winget
├─ scripts/smoke-test.ps1         End-to-end engine test (still + 1080p60 + 4K60)
└─ .github/workflows/windows.yml  CI: tests + Windows installer build
```

---

## 1. Install dependencies (Windows 10/11, x64)

Open **PowerShell** and run (one time):

```powershell
git clone https://github.com/novaaurora291-ops/MyLoader.git
cd MyLoader
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\setup-windows.ps1
```

This installs, via `winget`:

| Tool | Why | Manual command |
|---|---|---|
| Visual Studio 2022 Build Tools (C++) | Rust MSVC linker | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| Rust (rustup, stable-msvc) | engine + Tauri | `winget install Rustlang.Rustup` then `rustup default stable-msvc` |
| Node.js LTS (≥ 20.19) | UI build | `winget install OpenJS.NodeJS.LTS` |
| FFmpeg (full build, has NVENC/AMF/QSV) | decode / encode | `winget install Gyan.FFmpeg` |
| WebView2 Runtime | Tauri window (preinstalled on Win 11) | `winget install Microsoft.EdgeWebView2Runtime` |

**Close and reopen the terminal**, then check:

```powershell
rustc --version
node --version
ffmpeg -version
```

GPU drivers: keep NVIDIA / AMD / Intel drivers up to date — NVENC/AMF/QSV come from the driver.

> FFmpeg is found on `PATH`, next to the app (`ffmpeg.exe` beside `MyLoader.exe` or in a `ffmpeg\` sub-folder), or via `MYLOADER_FFMPEG=C:\path\to\ffmpeg\bin`.

---

## 2. Build & run

```powershell
cd MyLoader
npm install

# Development (hot reload UI + Rust backend)
npm run tauri dev

# Release build + Windows installer (.exe NSIS + .msi)
npm run tauri build
```

Outputs:
- `target\release\myloader.exe` — the app
- `target\release\bundle\nsis\MyLoader_0.1.0_x64-setup.exe` — installer
- `target\release\bundle\msi\MyLoader_0.1.0_x64_en-US.msi`

UI-only (no Rust, browser preview, export disabled): `npm run dev` → http://localhost:1420

---

## 3. Using the app

1. **＋ Video** — pick your raw video; it goes on the bottom track and is auto-filled to the canvas.
2. **＋ AM Preset (.xml)** — import an Alight Motion preset (try `presets\hdr-cc-lyrics.xml`). Its layers stack above the video, rescaled to your canvas.
3. Or **＋ Adjustment** / **＋ Lyrics Fade** / **＋ Rectangle**.
4. Select a layer → move the sliders in the right panel; preview updates live on the GPU.
5. **9:16 / 16:9 / 1:1 / 4:5** → change canvas, then **⤢ Fill All Layers to Composition**.
6. Timeline: 👁 hide, 🔒 lock, drag ⠿ to reorder, drag clips to move in time, click the ruler to scrub.
7. **Export ⇪** → 1080p / 4K, 60 fps, encoder Auto (NVENC → AMF → QSV → x264).

Shortcuts: `Space` play/pause · `Ctrl+Z` / `Ctrl+Y` undo/redo · `Ctrl+S` save · `Del` delete layer · `H` hide · `L` lock.
Projects save as `.mlproj` (JSON, same format the CLI uses).

---

## 4. Test

```powershell
# Unit tests: AM parser, layout/fill, blend maths, CC maths, encoder args, WGSL shader validation
cargo test -p myloader-engine

# Frontend type-check
npm run typecheck

# Full end-to-end on your PC: GPU info, tested encoders, preset parse, still frame,
# 4K GPU benchmark, 1080p60 and 4K60 hardware exports (outputs in test-output\)
powershell -ExecutionPolicy Bypass -File scripts\smoke-test.ps1
```

### Engine CLI (`mlcli`)

```powershell
cargo build --release -p myloader-engine
$cli = "target\release\mlcli.exe"

& $cli info                                        # FFmpeg, GPU adapter, which HW encoders really work
& $cli parse presets\hdr-cc-lyrics.xml             # inspect an Alight Motion preset (--json for full dump)
& $cli new my.mp4 -o my.mlproj --aspect 9:16 --preset presets\hdr-cc-lyrics.xml
& $cli fill my.mlproj --aspect 16:9 --fit cover    # change aspect + Fill All Layers
& $cli still my.mlproj -o frame.png --t 3 --res 1080
& $cli export my.mlproj -o out_4k60.mp4 --res 2160 --fps 60 --encoder nvenc
& $cli export my.mlproj -o out.mp4 --encoder amf   # or qsv, hevc_nvenc, hevc_amf, hevc_qsv, cpu
& $cli bench --res 2160 --frames 120               # renderer throughput
```

Flags: `--res 1080|1440|2160|native`, `--fps`, `--encoder auto|nvenc|hevc_nvenc|amf|hevc_amf|qsv|hevc_qsv|cpu`, `--quality 19` (lower = better), `--bitrate 40000` (kbps), `--no-audio`, `--cpu` (CPU renderer).

---

## How it works

**Render graph (per frame)** — two `rgba16float` ping-pong accumulators:
- video / rectangle layer → `fs_layer`: inverse-transform to layer UV, bilinear sample (or rounded-rect SDF), blend mode + opacity onto the composite;
- adjustment layer → optional separable Gaussian `fs_blur` (H then V) of the composite below → `fs_adjust`: exposure → brightness → contrast → HDR highlights/shadows (luma-weighted) → saturation → vignette → bottom fade, blended back with the layer's blend mode/opacity inside its rectangle;
- `fs_present` → `rgba8unorm` → readback → encoder.

**Export pipeline** — decoder threads (`ffmpeg -hwaccel auto … -f rawvideo rgba`, decoded directly at display size) → bounded channel → wgpu render → bounded channel → encoder thread (`ffmpeg -f rawvideo -i - -c:v h264_nvenc …`, BT.709 tagged, `+faststart`, AAC audio from the video layer).

**Encoder settings** — NVENC `-preset p5 -tune hq -rc vbr -cq N` + spatial/temporal AQ; AMF `-quality quality -rc cqp`; QSV `-preset slower -global_quality N` (NV12); x264 `-crf N`.

## Troubleshooting

| Problem | Fix |
|---|---|
| `ffmpeg not found` | `winget install Gyan.FFmpeg`, reopen terminal, or set `MYLOADER_FFMPEG` |
| Encoder shows "not available" | update GPU driver; NVENC needs GTX 900+/RTX, AMF a Radeon, QSV an Intel iGPU enabled in BIOS |
| `GPU renderer unavailable` | update drivers; try `set WGPU_BACKEND=vulkan` or `dx12`; CPU fallback still works |
| `link.exe not found` | install VS 2022 Build Tools with "Desktop development with C++" |
| Preview black but export fine | the app automatically retries the preview with an in-memory copy of the video; very large files may take a moment |
| Vite/Node error | Node ≥ 20.19 required: `winget upgrade OpenJS.NodeJS.LTS` |

## Notes on Alight Motion compatibility

Alight Motion has 200+ effects; MyLoader maps the colour-grading core used by HDR CC presets (see list above). Keyframed values use the **first keyframe**; nested scenes are flattened; text/drawing/media placeholder layers are skipped with a warning (you import your own video as the bottom layer). Exact pixel parity with the mobile app is not guaranteed — tune with the sliders after import.

License: MIT
