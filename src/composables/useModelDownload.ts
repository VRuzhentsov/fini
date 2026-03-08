import { ref, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface DownloadProgress {
  file: string;
  file_index: number;
  file_count: number;
  percent: number; // 0–100, or -1 if unknown
  done: boolean;
  error: string | null;
}

export function useModelDownload() {
  const downloading = ref(false);
  const downloaded = ref(false);
  const progress = ref<DownloadProgress | null>(null);
  const error = ref<string | null>(null);

  let unlisten: UnlistenFn | null = null;

  async function checkDownloaded() {
    downloaded.value = await invoke<boolean>("model_downloaded");
  }

  async function startDownload() {
    if (downloading.value) return;
    downloading.value = true;
    error.value = null;
    progress.value = null;

    unlisten = await listen<DownloadProgress>("model://download-progress", (e) => {
      progress.value = e.payload;
      if (e.payload.done) {
        downloaded.value = true;
        downloading.value = false;
      }
      if (e.payload.error) {
        error.value = e.payload.error;
        downloading.value = false;
      }
    });

    try {
      await invoke("download_asr_model");
    } catch (e) {
      error.value = String(e);
      downloading.value = false;
    } finally {
      unlisten?.();
      unlisten = null;
    }
  }

  onUnmounted(() => {
    unlisten?.();
  });

  return { downloading, downloaded, progress, error, checkDownloaded, startDownload };
}
