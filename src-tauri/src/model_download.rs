use futures_util::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const HF_BASE: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-small-en-2023-06-26/resolve/main";

const MODEL_FILES: &[&str] = &[
    "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    "tokens.txt",
];

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub percent: i32,
    pub done: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn download_asr_model(app: AppHandle) -> Result<(), String> {
    let dest_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models")
        .join("asr");

    tokio::fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| e.to_string())?;

    let client = reqwest::Client::builder()
        .user_agent("fini-app/0.1")
        .build()
        .map_err(|e| e.to_string())?;

    let file_count = MODEL_FILES.len();

    for (idx, filename) in MODEL_FILES.iter().enumerate() {
        let dest = dest_dir.join(filename);

        if dest.exists()
            && tokio::fs::metadata(&dest)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
                > 1024
        {
            emit_progress(&app, filename, idx, file_count, 100, false, None);
            continue;
        }

        let url = format!("{HF_BASE}/{filename}");
        if let Err(e) = download_file(&app, &client, &url, &dest, filename, idx, file_count).await
        {
            let msg = format!("Failed to download {filename}: {e}");
            emit_progress(&app, filename, idx, file_count, 0, false, Some(msg.clone()));
            return Err(msg);
        }
    }

    emit_progress(&app, "all", file_count, file_count, 100, true, None);
    Ok(())
}

async fn download_file(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    dest: &PathBuf,
    label: &str,
    idx: usize,
    total: usize,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    let content_length = resp.content_length();
    let mut downloaded: u64 = 0;

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;

    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| e.to_string())?;
        file.write_all(&bytes).await.map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;

        let percent = content_length
            .map(|len| ((downloaded as f64 / len as f64) * 100.0) as i32)
            .unwrap_or(-1);

        emit_progress(app, label, idx, total, percent, false, None);
    }

    file.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn emit_progress(
    app: &AppHandle,
    file: &str,
    file_index: usize,
    file_count: usize,
    percent: i32,
    done: bool,
    error: Option<String>,
) {
    let _ = app.emit(
        "model://download-progress",
        DownloadProgress {
            file: file.to_string(),
            file_index,
            file_count,
            percent,
            done,
            error,
        },
    );
}
