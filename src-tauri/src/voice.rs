use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use sherpa_onnx::{
    OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineTransducerModelConfig,
};
use sherpa_onnx_sys::FeatureConfig;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct VoiceState(pub Mutex<Option<ActiveSession>>);

// cpal::Stream wraps platform raw pointers. We manage its lifetime carefully
// (always behind a Mutex, dropped on stop), so cross-thread send is safe.
unsafe impl Send for VoiceState {}
unsafe impl Sync for VoiceState {}

pub struct ActiveSession {
    _stream: cpal::Stream,
    stop_tx: std::sync::mpsc::SyncSender<()>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct TranscriptEvent {
    pub text: String,
    pub is_final: bool,
}

// ---------------------------------------------------------------------------
// Model paths
// ---------------------------------------------------------------------------

pub struct ModelPaths {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
}

impl ModelPaths {
    pub fn from_app_data(app: &AppHandle) -> Result<Self, String> {
        let base = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("models")
            .join("asr");

        let p = |name: &str| -> Result<String, String> {
            base.join(name)
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path for {name}"))
                .map(|s| s.to_string())
        };

        Ok(Self {
            encoder: p("encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?,
            decoder: p("decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?,
            joiner:  p("joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?,
            tokens:  p("tokens.txt")?,
        })
    }

    pub fn all_exist(&self) -> bool {
        [&self.encoder, &self.decoder, &self.joiner, &self.tokens]
            .iter()
            .all(|p| std::path::Path::new(p).exists())
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_recognition(
    app: AppHandle,
    state: State<VoiceState>,
) -> Result<(), String> {
    let mut guard = state.inner().0.lock().unwrap();
    if guard.is_some() {
        return Err("Recognition already running".into());
    }

    let paths = ModelPaths::from_app_data(&app)?;
    if !paths.all_exist() {
        return Err(
            "ASR model not found. Go to Settings → Download Voice Model first.".into(),
        );
    }

    let (stop_tx, stop_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("No microphone found")?;

    let stream_config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16_000),
        buffer_size: cpal::BufferSize::Default,
    };

    let tx = audio_tx.clone();
    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _| { let _ = tx.try_send(data.to_vec()); },
            |err| eprintln!("[voice] cpal error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;

    let app_handle = app.clone();
    let encoder = paths.encoder.clone();
    let decoder = paths.decoder.clone();
    let joiner  = paths.joiner.clone();
    let tokens  = paths.tokens.clone();

    std::thread::spawn(move || {
        recognition_worker(app_handle, encoder, decoder, joiner, tokens, audio_rx, stop_rx);
    });

    *guard = Some(ActiveSession { _stream: stream, stop_tx });
    Ok(())
}

#[tauri::command]
pub fn stop_recognition(state: State<VoiceState>) -> Result<(), String> {
    if let Some(session) = state.inner().0.lock().unwrap().take() {
        let _ = session.stop_tx.try_send(());
    }
    Ok(())
}

#[tauri::command]
pub fn model_downloaded(app: AppHandle) -> Result<bool, String> {
    Ok(ModelPaths::from_app_data(&app)?.all_exist())
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn recognition_worker(
    app: AppHandle,
    encoder: String,
    decoder: String,
    joiner: String,
    tokens: String,
    audio_rx: std::sync::mpsc::Receiver<Vec<f32>>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    let config = OnlineRecognizerConfig {
        feat_config: FeatureConfig {
            sample_rate: 16_000,
            feature_dim: 80,
        },
        model_config: OnlineModelConfig {
            transducer: OnlineTransducerModelConfig {
                encoder: Some(encoder),
                decoder: Some(decoder),
                joiner:  Some(joiner),
            },
            tokens: Some(tokens),
            num_threads: 2,
            provider: Some("cpu".to_string()),
            ..Default::default()
        },
        decoding_method: Some("greedy_search".to_string()),
        enable_endpoint: true,
        rule1_min_trailing_silence: 2.4,
        rule2_min_trailing_silence: 1.2,
        rule3_min_utterance_length: 20.0,
        ..Default::default()
    };

    let recognizer = match OnlineRecognizer::create(&config) {
        Some(r) => r,
        None => {
            eprintln!("[voice] failed to create recognizer");
            let _ = app.emit("voice://error", "Failed to load ASR model");
            return;
        }
    };

    let stream = recognizer.create_stream();
    let mut last_text = String::new();

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        match audio_rx.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(samples) => {
                stream.accept_waveform(16_000, &samples);

                while recognizer.is_ready(&stream) {
                    recognizer.decode(&stream);
                }

                if let Some(result) = recognizer.get_result(&stream) {
                    let text = result.text.trim().to_string();
                    if !text.is_empty() && text != last_text {
                        let _ = app.emit(
                            "voice://transcript",
                            TranscriptEvent { text: text.clone(), is_final: false },
                        );
                        last_text = text;
                    }
                }

                if recognizer.is_endpoint(&stream) {
                    if !last_text.is_empty() {
                        let _ = app.emit(
                            "voice://transcript",
                            TranscriptEvent { text: last_text.clone(), is_final: true },
                        );
                    }
                    recognizer.reset(&stream);
                    last_text.clear();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if !last_text.is_empty() {
        let _ = app.emit(
            "voice://transcript",
            TranscriptEvent { text: last_text, is_final: true },
        );
    }
}
