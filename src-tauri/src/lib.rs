mod keys;
mod tinify;

use keys::{
    add_key, mark_exhausted, pick_available_key, public_keys_view, remove_key, set_active_key,
    KeysView,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tinify::{compress_file, TinifyError};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn ensure_dirs(app: &AppHandle) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let output_dir = data_dir.join("output");
    let zip_dir = data_dir.join("zips");
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&zip_dir).map_err(|e| e.to_string())?;
    Ok((config_dir, output_dir, zip_dir))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SwitchInfo {
    from: String,
    reason: String,
    masked: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileResult {
    ok: bool,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compression_count: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ZipInfo {
    path: String,
    name: String,
    count: usize,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompressResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    results: Vec<FileResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    zip: Option<ZipInfo>,
    switches: Vec<SwitchInfo>,
    keys: KeysView,
}

fn create_zip(files: &[PathBuf], zip_path: &Path) -> Result<u64, String> {
    let file = File::create(zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut used = HashSet::new();

    for path in files {
        if !path.is_file() {
            continue;
        }
        let mut name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        if used.contains(&name) {
            let p = Path::new(&name);
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| format!(".{s}"))
                .unwrap_or_default();
            let mut i = 2;
            loop {
                let candidate = format!("{stem}_{i}{ext}");
                if !used.contains(&candidate) {
                    name = candidate;
                    break;
                }
                i += 1;
            }
        }
        used.insert(name.clone());
        zip.start_file(name, options)
            .map_err(|e| e.to_string())?;
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(fs::metadata(zip_path).map(|m| m.len()).unwrap_or(0))
}

async fn compress_with_failover(
    config_dir: &Path,
    source: &Path,
    output_dir: &Path,
    format: Option<&str>,
) -> Result<(tinify::CompressOk, String, Vec<SwitchInfo>), String> {
    let mut tried = HashSet::new();
    let mut switches = Vec::new();

    loop {
        let Some(entry) = pick_available_key(config_dir, &tried) else {
            return Err("所有 API Key 本月额度已用完，请添加新 Key 或等待每月刷新".into());
        };

        match compress_file(&entry.key, source, output_dir, format).await {
            Ok(info) => {
                let _ = set_active_key(config_dir, &entry.id);
                return Ok((info, entry.id, switches));
            }
            Err(TinifyError::QuotaExceeded) => {
                mark_exhausted(config_dir, &entry.id);
                tried.insert(entry.id.clone());
                switches.push(SwitchInfo {
                    from: entry.id.clone(),
                    reason: "额度用尽".into(),
                    masked: keys::mask_key(&entry.key),
                });
            }
            Err(TinifyError::Message(msg)) => return Err(msg),
        }
    }
}

#[tauri::command]
fn list_keys(app: AppHandle) -> Result<KeysView, String> {
    let (config_dir, _, _) = ensure_dirs(&app)?;
    Ok(public_keys_view(&config_dir))
}

#[tauri::command]
fn add_api_key(app: AppHandle, api_key: String, label: Option<String>) -> Result<KeysView, String> {
    let (config_dir, _, _) = ensure_dirs(&app)?;
    add_key(&config_dir, &api_key, label.as_deref().unwrap_or(""))
}

#[tauri::command]
fn remove_api_key(app: AppHandle, id: String) -> Result<KeysView, String> {
    let (config_dir, _, _) = ensure_dirs(&app)?;
    remove_key(&config_dir, &id)
}

#[tauri::command]
fn set_active_api_key(app: AppHandle, id: String) -> Result<KeysView, String> {
    let (config_dir, _, _) = ensure_dirs(&app)?;
    set_active_key(&config_dir, &id)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    index: usize,
    total: usize,
    name: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
}

#[tauri::command]
async fn compress_images(
    app: AppHandle,
    paths: Vec<String>,
    format: Option<String>,
    progress_offset: Option<usize>,
    progress_total: Option<usize>,
    skip_zip: Option<bool>,
) -> Result<CompressResponse, String> {
    let (config_dir, output_dir, zip_dir) = ensure_dirs(&app)?;
    let _ = public_keys_view(&config_dir);

    if pick_available_key(&config_dir, &HashSet::new()).is_none() {
        return Ok(CompressResponse {
            ok: false,
            error: Some("没有可用的 API Key（全部用尽或未添加）".into()),
            results: Vec::new(),
            zip: None,
            switches: Vec::new(),
            keys: public_keys_view(&config_dir),
        });
    }

    let fmt = format
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let offset = progress_offset.unwrap_or(0);
    let path_count = paths.len();
    let total = progress_total.unwrap_or(path_count).max(1);
    let mut results = Vec::new();
    let mut success_paths = Vec::new();
    let mut all_switches = Vec::new();

    for (local_index, path_str) in paths.into_iter().enumerate() {
        let index = offset + local_index;
        let source = PathBuf::from(&path_str);
        let name = source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&path_str)
            .to_string();

        let _ = app.emit(
            "compress-progress",
            ProgressEvent {
                index,
                total,
                name: name.clone(),
                status: "start".into(),
                error: None,
                input_size: None,
                output_size: None,
                ratio: None,
                output_path: None,
            },
        );

        match compress_with_failover(&config_dir, &source, &output_dir, fmt.as_deref()).await {
            Ok((info, _key_id, switches)) => {
                all_switches.extend(switches);
                success_paths.push(info.output.clone());
                let result = FileResult {
                    ok: true,
                    name: name.clone(),
                    error: None,
                    input_size: Some(info.input_size),
                    output_size: Some(info.output_size),
                    saved: Some(info.saved),
                    ratio: Some(info.ratio),
                    output_path: Some(info.output.to_string_lossy().into()),
                    output_type: Some(info.output_type),
                    compression_count: info.compression_count,
                };
                let _ = app.emit(
                    "compress-progress",
                    ProgressEvent {
                        index,
                        total,
                        name,
                        status: "ok".into(),
                        error: None,
                        input_size: result.input_size,
                        output_size: result.output_size,
                        ratio: result.ratio,
                        output_path: result.output_path.clone(),
                    },
                );
                results.push(result);
            }
            Err(err) => {
                let _ = app.emit(
                    "compress-progress",
                    ProgressEvent {
                        index,
                        total,
                        name: name.clone(),
                        status: "fail".into(),
                        error: Some(err.clone()),
                        input_size: None,
                        output_size: None,
                        ratio: None,
                        output_path: None,
                    },
                );
                results.push(FileResult {
                    ok: false,
                    name,
                    error: Some(err),
                    input_size: None,
                    output_size: None,
                    saved: None,
                    ratio: None,
                    output_path: None,
                    output_type: None,
                    compression_count: None,
                });
            }
        }
    }

    let zip = if skip_zip.unwrap_or(false) || success_paths.is_empty() {
        None
    } else {
        let stamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
        let zip_name = format!("compressed-{stamp}.zip");
        let zip_path = zip_dir.join(&zip_name);
        match create_zip(&success_paths, &zip_path) {
            Ok(bytes) => Some(ZipInfo {
                path: zip_path.to_string_lossy().into(),
                name: zip_name,
                count: success_paths.len(),
                bytes,
            }),
            Err(_) => None,
        }
    };

    Ok(CompressResponse {
        ok: true,
        error: None,
        results,
        zip,
        switches: all_switches,
        keys: public_keys_view(&config_dir),
    })
}

#[tauri::command]
fn zip_paths(app: AppHandle, paths: Vec<String>) -> Result<Option<ZipInfo>, String> {
    let (_, _, zip_dir) = ensure_dirs(&app)?;
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    if files.is_empty() {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
    let zip_name = format!("compressed-{stamp}.zip");
    let zip_path = zip_dir.join(&zip_name);
    let bytes = create_zip(&files, &zip_path)?;
    Ok(Some(ZipInfo {
        path: zip_path.to_string_lossy().into(),
        name: zip_name,
        count: files.len(),
        bytes,
    }))
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("文件不存在".into());
    }
    opener::reveal(&p).map_err(|e| e.to_string())
}

#[tauri::command]
fn copy_zip_to(source: String, dest: String) -> Result<(), String> {
    let src = PathBuf::from(source);
    let dst = PathBuf::from(dest);
    if !src.is_file() {
        return Err("压缩包不存在".into());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&src, &dst).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_output_dir(app: AppHandle) -> Result<String, String> {
    let (_, output_dir, _) = ensure_dirs(&app)?;
    opener::open(&output_dir).map_err(|e| e.to_string())?;
    Ok(output_dir.to_string_lossy().into())
}

// Use tauri-plugin-opener's reveal via JS instead; keep a tiny opener dep-free reveal using std::process
mod opener {
    use std::path::Path;
    use std::process::Command;

    pub fn open(path: &Path) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            Command::new("explorer")
                .arg(path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Command::new("xdg-open")
                .arg(path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    pub fn reveal(path: &Path) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            Command::new("explorer")
                .args(["/select,", &path.to_string_lossy()])
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .args(["-R", &path.to_string_lossy()])
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(parent) = path.parent() {
                open(parent)
            } else {
                open(path)
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            fs::create_dir_all(&config_dir)?;
            // 启动时做月度刷新检测
            let _ = public_keys_view(&config_dir);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_keys,
            add_api_key,
            remove_api_key,
            set_active_api_key,
            compress_images,
            zip_paths,
            reveal_path,
            copy_zip_to,
            open_output_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
