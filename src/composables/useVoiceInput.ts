/// Voice input via sherpa-onnx (on-device, offline).
/// Calls Rust commands start_recognition / stop_recognition and listens
/// for `voice://transcript` events emitted by the worker thread.

import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface TranscriptEvent {
  text: string;
  is_final: boolean;
}

export function useVoiceInput() {
  const transcript = ref("");
  const listening = ref(false);
  const error = ref<string | null>(null);

  let unlisten: UnlistenFn | null = null;

  async function start(): Promise<void> {
    error.value = null;
    transcript.value = "";

    // Subscribe to transcript events before starting recognition
    unlisten = await listen<TranscriptEvent>("voice://transcript", (event) => {
      transcript.value = event.payload.text;
    });

    try {
      await invoke("start_recognition");
      listening.value = true;
    } catch (e) {
      unlisten?.();
      unlisten = null;
      error.value = String(e);
      throw e;
    }
  }

  async function stop(): Promise<string> {
    try {
      await invoke("stop_recognition");
    } finally {
      listening.value = false;
      unlisten?.();
      unlisten = null;
    }
    return transcript.value;
  }

  return { transcript, listening, error, start, stop };
}
