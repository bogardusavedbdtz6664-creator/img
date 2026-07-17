use reqwest::header::{CONTENT_TYPE, LOCATION};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct CompressOk {
    pub output: PathBuf,
    pub input_size: u64,
    pub output_size: u64,
    pub saved: u64,
    pub ratio: f64,
    pub output_type: String,
    pub compression_count: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum TinifyError {
    #[error("额度用尽")]
    QuotaExceeded,
    #[error("{0}")]
    Message(String),
}

fn is_quota_status(status: reqwest::StatusCode, body: &str) -> bool {
    if status.as_u16() == 429 {
        return true;
    }
    let lower = body.to_lowercase();
    lower.contains("monthly limit")
        || lower.contains("limit has been exceeded")
        || lower.contains("exceeded your")
        || lower.contains("too many requests")
}

fn mime_for_format(fmt: &str) -> Option<&'static str> {
    match fmt {
        "webp" => Some("image/webp"),
        "jpeg" | "jpg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        _ => None,
    }
}

fn ext_for_mime(mime: &str, fallback: &str) -> String {
    match mime {
        "image/webp" => ".webp".into(),
        "image/jpeg" => ".jpg".into(),
        "image/png" => ".png".into(),
        "image/avif" => ".avif".into(),
        _ => {
            let e = Path::new(fallback)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| format!(".{s}"))
                .unwrap_or_else(|| ".bin".into());
            if e.eq_ignore_ascii_case(".jpeg") {
                ".jpg".into()
            } else {
                e.to_ascii_lowercase()
            }
        }
    }
}

pub async fn compress_file(
    api_key: &str,
    source: &Path,
    output_dir: &Path,
    target_format: Option<&str>,
) -> Result<CompressOk, TinifyError> {
    if !source.is_file() {
        return Err(TinifyError::Message(format!("文件不存在: {}", source.display())));
    }
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "avif") {
        return Err(TinifyError::Message(format!(
            "不支持的格式: .{ext}（支持 PNG / JPEG / WEBP / AVIF）"
        )));
    }

    let input_bytes = tokio::fs::read(source)
        .await
        .map_err(|e| TinifyError::Message(e.to_string()))?;
    let input_size = input_bytes.len() as u64;

    let mut fmt = target_format
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "original");
    if fmt.as_deref() == Some("jpg") {
        fmt = Some("jpeg".into());
    }
    if let Some(f) = &fmt {
        if mime_for_format(f).is_none() {
            return Err(TinifyError::Message(format!(
                "无效的目标格式: {f}（可选: webp / jpeg / png）"
            )));
        }
    }

    let client = reqwest::Client::new();
    let shrink = client
        .post("https://api.tinify.com/shrink")
        .basic_auth("api", Some(api_key))
        .header(CONTENT_TYPE, "application/octet-stream")
        .body(input_bytes)
        .send()
        .await
        .map_err(|e| TinifyError::Message(e.to_string()))?;

    let status = shrink.status();
    let compression_count = shrink
        .headers()
        .get("Compression-Count")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    let location = shrink
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body_text = shrink.text().await.unwrap_or_default();

    if is_quota_status(status, &body_text) {
        return Err(TinifyError::QuotaExceeded);
    }
    if !status.is_success() && status.as_u16() != 201 {
        return Err(TinifyError::Message(format!(
            "压缩失败 (HTTP {}): {}",
            status.as_u16(),
            body_text.chars().take(200).collect::<String>()
        )));
    }
    let location = location.ok_or_else(|| TinifyError::Message("API 未返回输出地址".into()))?;

    let output_resp = if let Some(f) = &fmt {
        let mime = mime_for_format(f).unwrap();
        let mut payload = json!({ "convert": { "type": mime } });
        if f == "jpeg" {
            payload["transform"] = json!({ "background": "#ffffff" });
        }
        client
            .post(&location)
            .basic_auth("api", Some(api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| TinifyError::Message(e.to_string()))?
    } else {
        client
            .get(&location)
            .basic_auth("api", Some(api_key))
            .send()
            .await
            .map_err(|e| TinifyError::Message(e.to_string()))?
    };

    let out_status = output_resp.status();
    let out_type = output_resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let out_bytes = output_resp
        .bytes()
        .await
        .map_err(|e| TinifyError::Message(e.to_string()))?;

    if is_quota_status(out_status, &String::from_utf8_lossy(&out_bytes)) {
        return Err(TinifyError::QuotaExceeded);
    }
    if !out_status.is_success() {
        return Err(TinifyError::Message(format!(
            "下载结果失败 (HTTP {})",
            out_status.as_u16()
        )));
    }

    tokio::fs::create_dir_all(output_dir)
        .await
        .map_err(|e| TinifyError::Message(e.to_string()))?;

    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let out_ext = ext_for_mime(&out_type, source.to_string_lossy().as_ref());
    let out_path = output_dir.join(format!("{stem}{out_ext}"));
    tokio::fs::write(&out_path, &out_bytes)
        .await
        .map_err(|e| TinifyError::Message(e.to_string()))?;

    let output_size = out_bytes.len() as u64;
    let saved = input_size.saturating_sub(output_size);
    let ratio = if input_size > 0 {
        ((saved as f64 / input_size as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    Ok(CompressOk {
        output: out_path,
        input_size,
        output_size,
        saved,
        ratio,
        output_type: out_type,
        compression_count,
    })
}
