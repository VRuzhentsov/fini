<script setup lang="ts">
import { ref, watch } from "vue";
import { useQuestStore } from "../stores/quest";
import { useVoiceInput } from "../composables/useVoiceInput";
import { useToast } from "../composables/useToast";

const store = useQuestStore();
const voice = useVoiceInput();
const toast = useToast();

watch(() => voice.error.value, (err) => { if (err) toast.error(err); });

type Phase = "idle" | "recording";
const phase = ref<Phase>("idle");
const title = ref("");
const submitting = ref(false);

async function submitText() {
  if (!title.value.trim()) return;
  submitting.value = true;
  try {
    await store.createQuest(title.value.trim());
    await store.fetchActiveQuest();
    title.value = "";
  } finally {
    submitting.value = false;
  }
}

// Push-to-talk: hold FAB to record, release to append transcript
async function startVoice(e: PointerEvent) {
  if (phase.value === "recording") return;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  try {
    await voice.start();
    phase.value = "recording";
  } catch (err) {
    toast.error(String(err));
  }
}

async function stopVoice() {
  if (phase.value !== "recording") return;
  phase.value = "idle";
  const transcript = await voice.stop();
  if (transcript) {
    title.value = title.value ? `${title.value} ${transcript}` : transcript;
  }
}
</script>

<template>
  <div class="new-quest">
    <h2>What's the quest?</h2>
    <form @submit.prevent="submitText" class="form">
      <input
        v-model="title"
        placeholder="Describe your quest…"
        class="input"
      />
      <button
        type="submit"
        class="btn-primary"
        :disabled="submitting || !title.trim()"
      >
        Start
      </button>
    </form>
  </div>

  <!-- PTT FAB -->
  <button
    class="fab"
    :class="{ 'fab--recording': phase === 'recording' }"
    @pointerdown.prevent="startVoice($event)"
    @pointerup="stopVoice"
    @pointercancel="stopVoice"
    @contextmenu.prevent
    title="Hold to speak"
  >🎙</button>

  <!-- live transcript while held -->
  <div v-if="phase === 'recording'" class="recording-overlay">
    <p class="live-transcript">{{ voice.transcript.value || "Listening…" }}</p>
  </div>

</template>

<style scoped>
.new-quest h2 {
  margin-bottom: 1rem;
  font-size: 1.25rem;
}

.form {
  display: flex;
  gap: 0.5rem;
  align-items: center;
}

.input {
  flex: 1;
  padding: 0.6rem 0.9rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.3);
  background: transparent;
  font-size: 1rem;
  font-family: inherit;
  color: inherit;
}

.btn-primary {
  padding: 0.6rem 1.2rem;
  border-radius: 6px;
  border: none;
  background: #646cff;
  color: white;
  cursor: pointer;
  font-size: 1rem;
}

.btn-primary:disabled { opacity: 0.4; cursor: default; }

.fab {
  position: fixed;
  bottom: calc(1.5rem + env(safe-area-inset-bottom));
  right: 1.5rem;
  width: 56px;
  height: 56px;
  border-radius: 50%;
  border: none;
  background: #646cff;
  color: white;
  font-size: 1.5rem;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  z-index: 100;
  user-select: none;
  -webkit-user-select: none;
  touch-action: none;
  transition: background 0.15s, transform 0.1s;
}

.fab--recording {
  background: #ef4444;
  transform: scale(1.15);
  box-shadow: 0 0 0 8px rgba(239, 68, 68, 0.25);
}

.recording-overlay {
  position: fixed;
  bottom: calc(5rem + env(safe-area-inset-bottom));
  right: 1rem;
  background: rgba(0, 0, 0, 0.7);
  color: white;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  font-size: 0.875rem;
  max-width: 220px;
  z-index: 99;
}

.live-transcript {
  margin: 0;
  opacity: 0.9;
}

.voice-error {
  position: fixed;
  bottom: calc(5rem + env(safe-area-inset-bottom));
  left: 1rem;
  right: 5rem;
  background: rgba(239, 68, 68, 0.9);
  color: white;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  font-size: 0.8rem;
  z-index: 99;
}
</style>
