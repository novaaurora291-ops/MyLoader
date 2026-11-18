// Thin wrapper around the Tauri commands in src-tauri/src/main.rs.
// When the UI runs in a plain browser (`npm run dev`) the native bits are
// unavailable; the app still works for preview / layout / CC.
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { EncoderInfo, ExportProgress, ExportReport, ExportSettings, ParsedPreset, Project, VideoInfo } from "../types";

export const isTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function pickVideo(): Promise<string | null> {
  const r = await open({
    multiple: false,
    filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "m4v", "avi"] }],
  });
  return typeof r === "string" ? r : null;
}

export async function pickPreset(): Promise<string | null> {
  const r = await open({ multiple: false, filters: [{ name: "Alight Motion XML", extensions: ["xml"] }] });
  return typeof r === "string" ? r : null;
}

export async function pickProjectOpen(): Promise<string | null> {
  const r = await open({ multiple: false, filters: [{ name: "MyLoader project", extensions: ["mlproj", "json"] }] });
  return typeof r === "string" ? r : null;
}

export async function pickProjectSave(): Promise<string | null> {
  return await save({ defaultPath: "project.mlproj", filters: [{ name: "MyLoader project", extensions: ["mlproj"] }] });
}

export async function pickExportPath(): Promise<string | null> {
  return await save({ defaultPath: "export.mp4", filters: [{ name: "MP4 video", extensions: ["mp4"] }] });
}

export const probeVideo = (path: string) => invoke<VideoInfo>("probe_video", { path });
export const parsePreset = (path: string) => invoke<ParsedPreset>("parse_preset", { path });
export const parsePresetText = (xml: string) => invoke<ParsedPreset>("parse_preset_text", { xml });
export const detectEncoders = () => invoke<EncoderInfo[]>("detect_encoders");
export const engineInfo = () => invoke<{ ffmpeg: string; gpu: string }>("engine_info");
export const saveProject = (path: string, project: Project) => invoke<void>("save_project", { path, project });
export const loadProject = (path: string) => invoke<Project>("load_project", { path });
export const startExport = (project: Project, settings: ExportSettings) => invoke<void>("start_export", { project, settings });
export const cancelExport = () => invoke<void>("cancel_export");

export function onExportEvents(handlers: {
  progress: (p: ExportProgress) => void;
  done: (r: ExportReport) => void;
  error: (msg: string) => void;
}): Promise<UnlistenFn[]> {
  return Promise.all([
    listen<ExportProgress>("export-progress", (e) => handlers.progress(e.payload)),
    listen<ExportReport>("export-done", (e) => handlers.done(e.payload)),
    listen<string>("export-error", (e) => handlers.error(e.payload)),
  ]);
}

/**
 * URL the <video> element can play AND upload into WebGL. The asset protocol
 * streams the file (no RAM copy) and sends CORS headers; if the WebView still
 * refuses the texture upload we fall back to an in-memory Blob URL.
 */
export function videoUrl(path: string): string {
  return convertFileSrc(path);
}

export async function videoBlobUrl(path: string): Promise<string> {
  const buf = await invoke<ArrayBuffer>("read_file_bytes", { path });
  return URL.createObjectURL(new Blob([buf], { type: "video/mp4" }));
}
