<script setup lang="ts">
import { onMounted } from "vue";
import { useModelDownload } from "../composables/useModelDownload";

const model = useModelDownload();

onMounted(() => model.checkDownloaded());

function progressLabel(): string {
  const p = model.progress.value;
  if (!p) return "";
  const pct = p.percent >= 0 ? `${p.percent}%` : "…";
  return `Downloading ${p.file} (${p.file_index + 1}/${p.file_count}) ${pct}`;
}
</script>

<template>
  <div class="view">
    <h2>Settings</h2>

    <section class="section">
      <h3>Voice Model</h3>
      <p class="desc">
        On-device speech recognition via sherpa-onnx.<br />
        Model: <code>sherpa-onnx-streaming-zipformer-small-en</code> (~60 MB)
      </p>

      <div class="status">
        <span v-if="model.downloaded.value" class="badge ready">Ready</span>
        <span v-else class="badge missing">Not downloaded</span>
      </div>

      <div v-if="model.downloading.value" class="progress-row">
        <div class="progress-bar-track">
          <div
            class="progress-bar-fill"
            :style="{
              width:
                model.progress.value && model.progress.value.percent >= 0
                  ? `${model.progress.value.percent}%`
                  : '100%',
              opacity: model.progress.value?.percent === -1 ? 0.5 : 1,
            }"
          />
        </div>
        <span class="progress-label">{{ progressLabel() }}</span>
      </div>

      <p v-if="model.error.value" class="error">{{ model.error.value }}</p>

      <button
        class="btn-primary"
        :disabled="model.downloading.value || model.downloaded.value"
        @click="model.startDownload()"
      >
        {{ model.downloaded.value ? "Downloaded" : model.downloading.value ? "Downloading…" : "Download Model" }}
      </button>
    </section>
  </div>
</template>

<style scoped>
.view {
  padding: 1.5rem;
}

h2 {
  margin-bottom: 1.5rem;
}

.section {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  max-width: 480px;
}

h3 {
  font-size: 1rem;
  font-weight: 600;
}

.desc {
  font-size: 0.875rem;
  opacity: 0.7;
  line-height: 1.5;
}

.status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.badge {
  font-size: 0.75rem;
  padding: 0.2rem 0.6rem;
  border-radius: 4px;
  font-weight: 600;
}

.badge.ready   { background: #22c55e22; color: #22c55e; }
.badge.missing { background: #f59e0b22; color: #f59e0b; }

.progress-row {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.progress-bar-track {
  height: 6px;
  border-radius: 3px;
  background: rgba(128,128,128,0.2);
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  background: #646cff;
  border-radius: 3px;
  transition: width 0.2s;
}

.progress-label {
  font-size: 0.75rem;
  opacity: 0.6;
}

.error {
  color: #ef4444;
  font-size: 0.875rem;
}

.btn-primary {
  padding: 0.6rem 1.2rem;
  border-radius: 6px;
  border: none;
  background: #646cff;
  color: white;
  cursor: pointer;
  font-size: 0.9rem;
  align-self: flex-start;
}

.btn-primary:disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
