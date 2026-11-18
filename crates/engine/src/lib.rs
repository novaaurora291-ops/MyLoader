//! # MyLoader engine
//!
//! High-performance colour grading & compositing for Windows desktops:
//! * [`alight`]  - Alight Motion `.xml` preset parser / importer
//! * [`model`]   - project / layer / CC data model (JSON, shared with the UI)
//! * [`layout`]  - aspect ratio presets and "Fill All Layers to Composition"
//! * [`gpu`]     - wgpu renderer (DX12 / Vulkan), WGSL shaders
//! * [`cpu`]     - CPU reference renderer (fallback + tests)
//! * [`ffmpeg`]  - probing, NVENC / AMF / QSV detection, decode / encode pipes
//! * [`export`]  - multi-threaded decode -> render -> encode pipeline

pub mod alight;
pub mod cpu;
pub mod export;
pub mod ffmpeg;
pub mod gpu;
pub mod layout;
pub mod model;
pub mod render;

pub use export::{run_export, ExportProgress, ExportReport, ExportSettings};
pub use model::*;
