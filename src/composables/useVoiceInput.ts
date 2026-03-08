/// Voice input via sherpa-onnx (on-device, offline).

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
  let unlistenError: UnlistenFn | null = null;

  async function start(): Promise<void> {
    error.value = null;
    transcript.value = "";

    unlisten = await listen<TranscriptEvent>("voice://transcript", (event) => {
      transcript.value = event.payload.text;
    });

    unlistenError = await listen<string>("voice://error", (event) => {
      error.value = event.payload;
    });

    try {
      await invoke("start_recognition");
      listening.value = true;
    } catch (e) {
      unlisten?.(); unlisten = null;
      unlistenError?.(); unlistenError = null;
      error.value = String(e);
      throw e;
    }
  }

  async function stop(): Promise<string> {
    try {
      await invoke("stop_recognition");
      await new Promise(resolve => setTimeout(resolve, 300));
    } finally {
      listening.value = false;
      unlisten?.(); unlisten = null;
      unlistenError?.(); unlistenError = null;
    }
    return transcript.value;
  }

  return { transcript, listening, error, start, stop };
}
