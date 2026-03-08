<script setup lang="ts">
import { ref } from "vue";
import { useQuestStore } from "../stores/quest";
import { useVoiceInput } from "../composables/useVoiceInput";
import { useStepBreakdown } from "../composables/useStepBreakdown";

const store = useQuestStore();
const voice = useVoiceInput();
const { parseSteps } = useStepBreakdown();

// --- state ---
type Phase = "idle" | "recording" | "confirm";
const phase = ref<Phase>("idle");

const title = ref("");
const pendingSteps = ref<string[]>([]);
const submitting = ref(false);

// --- text submit ---
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

// --- voice flow ---
async function startVoice() {
  try {
    await voice.start();
    phase.value = "recording";
  } catch (e) {
    alert(`Voice error: ${e}`);
  }
}

async function stopVoice() {
  const transcript = await voice.stop();
  if (!transcript) {
    phase.value = "idle";
    return;
  }
  title.value = transcript;
  pendingSteps.value = parseSteps(transcript);
  phase.value = "confirm";
}

// --- confirm steps ---
function removeStep(i: number) {
  pendingSteps.value.splice(i, 1);
}

function addStep() {
  pendingSteps.value.push("");
}

async function confirmQuest() {
  if (!title.value.trim()) return;
  submitting.value = true;
  try {
    const quest = await store.createQuest(title.value.trim());
    for (let i = 0; i < pendingSteps.value.length; i++) {
      const body = pendingSteps.value[i].trim();
      if (body) await store.createStep(quest.id, body, i);
    }
    await store.fetchActiveQuest();
    reset();
  } finally {
    submitting.value = false;
  }
}

function reset() {
  phase.value = "idle";
  title.value = "";
  pendingSteps.value = [];
}
</script>

<template>
  <!-- IDLE: text input + mic button -->
  <div v-if="phase === 'idle'" class="new-quest">
    <h2>What's the quest?</h2>
    <div class="input-row">
      <form @submit.prevent="submitText" class="form">
        <input
          v-model="title"
          placeholder="Describe your quest…"
          class="input"
          autofocus
        />
        <button
          type="submit"
          class="btn-primary"
          :disabled="submitting || !title.trim()"
        >
          Start
        </button>
      </form>
      <button class="btn-mic" @click="startVoice" title="Voice input">
        🎙
      </button>
    </div>
  </div>

  <!-- RECORDING -->
  <div v-else-if="phase === 'recording'" class="recording">
    <div class="pulse" />
    <p class="live-transcript">
      {{ voice.transcript.value || "Listening…" }}
    </p>
    <button class="btn-secondary" @click="stopVoice">Done</button>
  </div>

  <!-- CONFIRM STEPS -->
  <div v-else-if="phase === 'confirm'" class="confirm">
    <h2>Confirm quest</h2>
    <input v-model="title" class="input title-input" />

    <p class="steps-label">Steps</p>
    <ul class="steps-list">
      <li v-for="(_, i) in pendingSteps" :key="i" class="step-item">
        <input v-model="pendingSteps[i]" class="input step-input" />
        <button class="btn-remove" @click="removeStep(i)">✕</button>
      </li>
    </ul>
    <button class="btn-ghost" @click="addStep">+ Add step</button>

    <div class="confirm-actions">
      <button class="btn-secondary" @click="reset">Cancel</button>
      <button
        class="btn-primary"
        :disabled="submitting || !title.trim()"
        @click="confirmQuest"
      >
        Start Quest
      </button>
    </div>
  </div>
</template>

<style scoped>
.new-quest h2,
.confirm h2 {
  margin-bottom: 1rem;
  font-size: 1.25rem;
}

.input-row {
  display: flex;
  gap: 0.5rem;
  align-items: flex-start;
}

.form {
  display: flex;
  gap: 0.5rem;
  flex: 1;
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

.title-input {
  width: 100%;
  margin-bottom: 1rem;
}

.step-input {
  flex: 1;
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

.btn-secondary {
  padding: 0.6rem 1rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.3);
  background: transparent;
  cursor: pointer;
  font-size: 0.9rem;
  color: inherit;
}

.btn-mic {
  padding: 0.55rem 0.75rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.3);
  background: transparent;
  cursor: pointer;
  font-size: 1.1rem;
}

.btn-ghost {
  background: none;
  border: none;
  color: #646cff;
  cursor: pointer;
  font-size: 0.875rem;
  padding: 0.25rem 0;
  text-align: left;
}

.btn-remove {
  background: none;
  border: none;
  cursor: pointer;
  opacity: 0.4;
  font-size: 0.75rem;
  padding: 0.2rem 0.4rem;
}

.btn-remove:hover { opacity: 1; }

/* Recording */
.recording {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  padding: 2rem 0;
}

.pulse {
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: #ef4444;
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { transform: scale(1); opacity: 1; }
  50%       { transform: scale(1.15); opacity: 0.7; }
}

.live-transcript {
  font-size: 1rem;
  opacity: 0.8;
  text-align: center;
  min-height: 1.5rem;
}

/* Confirm */
.confirm {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.steps-label {
  font-size: 0.8rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  opacity: 0.5;
  margin-top: 0.5rem;
}

.steps-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.step-item {
  display: flex;
  gap: 0.4rem;
  align-items: center;
}

.confirm-actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.5rem;
}
</style>
