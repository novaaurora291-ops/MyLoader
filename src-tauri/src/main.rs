// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use myloader_engine::{alight, export, ffmpeg, Project};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, State};

/// Shared app state: the cancel flag of the running export.
#[derive(Default)]
struct AppState {
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

#[tauri::command]
async fn probe_video(path: String) -> CmdResult<ffmpeg::VideoInfo> {
    tauri::async_runtime::spawn_blocking(move || ffmpeg::probe(&path)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
fn parse_preset(path: String) -> CmdResult<alight::ParsedPreset> {
    alight::parse_preset_file(std::path::Path::new(&path)).map_err(err)
}

#[tauri::command]
fn parse_preset_text(xml: String) -> CmdResult<alight::ParsedPreset> {
    alight::parse_preset_str(&xml).map_err(err)
}

#[tauri::command]
async fn detect_encoders() -> CmdResult<Vec<ffmpeg::EncoderInfo>> {
    tauri::async_runtime::spawn_blocking(ffmpeg::detect_encoders).await.map_err(err)
}

#[derive(serde::Serialize)]
struct EngineInfo {
    ffmpeg: String,
    gpu: String,
}

#[tauri::command]
async fn engine_info() -> CmdResult<EngineInfo> {
    tauri::async_runtime::spawn_blocking(|| {
        let ffmpeg = ffmpeg::ffmpeg_version().unwrap_or_else(|e| format!("not found: {e}"));
        let gpu = myloader_engine::gpu::GpuRenderer::new(myloader_engine::render::RenderSize { width: 16, height: 16, scale: 1.0 })
            .map(|g| g.adapter_name().to_string())
            .unwrap_or_else(|e| format!("unavailable: {e}"));
        EngineInfo { ffmpeg, gpu }
    })
    .await
    .map_err(err)
}

#[tauri::command]
fn save_project(path: String, project: Project) -> CmdResult<()> {
    std::fs::write(&path, project.to_json()).map_err(err)
}

#[tauri::command]
fn load_project(path: String) -> CmdResult<Project> {
    let s = std::fs::read_to_string(&path).map_err(err)?;
    Project::from_json(&s).map_err(err)
}

/// Fallback for the preview when the WebView refuses to upload an
/// asset-protocol video into WebGL: hand the bytes over as a Blob.
#[tauri::command]
fn read_file_bytes(path: String) -> CmdResult<tauri::ipc::Response> {
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(err)
}

#[tauri::command]
fn start_export(app: tauri::AppHandle, state: State<'_, AppState>, project: Project, settings: export::ExportSettings) -> CmdResult<()> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("an export is already running".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    std::thread::spawn(move || {
        let res = export::run_export(&project, &settings, &cancel, |p| {
            let _ = app.emit("export-progress", p);
        });
        busy.store(false, Ordering::SeqCst);
        match res {
            Ok(rep) => {
                let _ = app.emit("export-done", rep);
            }
            Err(e) => {
                let _ = app.emit("export-error", format!("{e:#}"));
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_export(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            probe_video,
            parse_preset,
            parse_preset_text,
            detect_encoders,
            engine_info,
            save_project,
            load_project,
            read_file_bytes,
            start_export,
            cancel_export
        ])
        .run(tauri::generate_context!())
        .expect("error while running MyLoader");
}
