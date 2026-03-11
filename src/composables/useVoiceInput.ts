/// Voice input via sherpa-onnx (on-device, offline).

import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface TranscriptEvent {
  text: string;
  is_final: boolean;
}

export function useVoiceInput() {
  const transcript = ref("");
  const listening = ref(false);
  const warming = ref(false);
  const error = ref<string | null>(null);

  let unlisteners: UnlistenFn[] = [];
  // Set to true when the user releases before start() resolves, so the
  // retry loop aborts and we skip committing the session.
  let startCancelled = false;

  // Register listeners once at mount — no per-press overhead
  onMounted(async () => {
    unlisteners.push(
      await listen<TranscriptEvent>("voice://transcript", (e) => {
        transcript.value = e.payload.text;
      }),
      await listen<string>("voice://error", (e) => {
        error.value = e.payload;
      }),
    );
  });

  onUnmounted(() => {
    unlisteners.forEach(fn => fn());
    unlisteners = [];
  });

  async function start(): Promise<void> {
    startCancelled = false;
    error.value = null;
    transcript.value = "";
    // Retry until the recognizer pool has a warmed instance (max 5s)
    for (let i = 0; i < 50; i++) {
      if (startCancelled) return;
      try {
        await invoke("start_recognition");
        if (startCancelled) {
          // User released while we were in the IPC call — clean up immediately
          invoke("stop_recognition").catch(() => {});
          return;
        }
        listening.value = true;
        warming.value = false;
        return;
      } catch (e) {
        if (String(e) !== "warming_up") throw e;
        warming.value = true;
        await new Promise(r => setTimeout(r, 100));
      }
    }
    warming.value = false;
    throw new Error("ASR warmup timed out");
  }

  async function stop(): Promise<string> {
    startCancelled = true;
    try {
      await invoke("stop_recognition");
      // Brief wait for the final transcript event from the worker thread
      await new Promise(resolve => setTimeout(resolve, 150));
    } finally {
      listening.value = false;
    }
    return transcript.value;
  }

  async function forceStop(): Promise<void> {
    startCancelled = true;
    try { await invoke("stop_recognition"); } catch {}
    await new Promise(resolve => setTimeout(resolve, 80));
    listening.value = false;
    transcript.value = "";
  }

  return { transcript, listening, warming, error, start, stop, forceStop };
}
