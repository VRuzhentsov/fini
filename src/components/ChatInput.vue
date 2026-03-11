<script setup lang="ts">
import { ref, watch, nextTick } from "vue";
import { useVoiceInput } from "../composables/useVoiceInput";
import { useToast } from "../composables/useToast";

const emit = defineEmits<{ submit: [text: string] }>();

const voice = useVoiceInput();
const { error: showError } = useToast();

watch(voice.error, (err) => { if (err) showError(err); });

const text = ref("");
const recording = ref(false);
const textarea = ref<HTMLTextAreaElement | null>(null);

function autoResize() {
  const el = textarea.value;
  if (!el) return;
  el.style.height = "auto";
  const lineHeight = parseInt(getComputedStyle(el).lineHeight);
  const maxHeight = lineHeight * 6;
  el.style.height = Math.min(el.scrollHeight, maxHeight) + "px";
}

function onSubmit() {
  const val = text.value.trim();
  if (!val) return;
  emit("submit", val);
  text.value = "";
  nextTick(autoResize);
}

// async function onMicPress(e: PointerEvent) {
//   (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
//   recording.value = true;
//   const err = await voice.start().catch(String);
//   if (!recording.value) { void voice.stop(); return; }
//   if (!err) return;
//   if (err.includes("already running")) {
//     await voice.forceStop();
//     await voice.start().catch(() => { recording.value = false; });
//   } else { showError(err); recording.value = false; }
// }

// function onMicRelease() {
//   recording.value = false;
//   void voice.stop().then(transcript => {
//     if (!transcript) return;
//     text.value = text.value ? `${text.value} ${transcript}` : transcript;
//   });
// }

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    onSubmit();
  }
}

</script>

<template>
  <div class="chat-wrap">
    <Transition name="transcript">
      <div v-if="recording" class="live-transcript">
        {{ voice.transcript.value || "Listening…" }}
      </div>
    </Transition>

    <form class="chat-bar" @submit.prevent="onSubmit">
      <textarea
        ref="textarea"
        v-model="text"
        class="chat-input"
        placeholder="Write a message…"
        rows="1"
        :readonly="recording"
        @input="autoResize"
        @keydown="onKeydown"
      />
      <button
        type="submit"
        class="icon-btn"
        :disabled="!text.trim()"
        aria-label="Send"
      >
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22">
          <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>
        </svg>
      </button>
      <!-- mic button: hidden until voice is re-enabled
      <button
        type="button"
        class="icon-btn"
        :class="{ 'icon-btn--recording': recording, 'icon-btn--warming': voice.warming.value }"
        @pointerdown.prevent="onMicPress($event)"
        @pointerup="onMicRelease"
        @pointercancel="onMicRelease"
        @contextmenu.prevent
        aria-label="Hold to speak"
      >
        <svg viewBox="0 0 24 24" fill="currentColor" width="22" height="22">
          <path d="M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.93V20H9v2h6v-2h-2v-2.07A7 7 0 0 0 19 11h-2z"/>
        </svg>
      </button>
      -->
    </form>
  </div>
</template>

<style scoped>
.chat-wrap {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  padding-bottom: env(safe-area-inset-bottom);
  background: #1e1e2e;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  z-index: 100;
}

.live-transcript {
  padding: 0.5rem 1rem;
  font-size: 0.875rem;
  color: rgba(255, 255, 255, 0.5);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.chat-bar {
  display: flex;
  align-items: center;
  padding: 0.5rem 0.75rem;
  gap: 0.25rem;
}

.chat-input {
  flex: 1;
  padding: 0.6rem 0.75rem;
  background: transparent;
  border: none;
  outline: none;
  font-size: 1rem;
  font-family: inherit;
  color: inherit;
  min-width: 0;
  resize: none;
  overflow-y: auto;
  line-height: 1.5;
}

.chat-input::placeholder {
  color: rgba(255, 255, 255, 0.3);
}

.icon-btn {
  flex-shrink: 0;
  width: 40px;
  height: 40px;
  border: none;
  background: transparent;
  color: rgba(255, 255, 255, 0.5);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  user-select: none;
  -webkit-user-select: none;
  touch-action: none;
  transition: color 0.15s;
}

.icon-btn:active { color: rgba(255, 255, 255, 0.9); }

.icon-btn--recording {
  color: #ef4444;
  animation: pulse-ring 1.2s ease-out infinite;
}

.icon-btn--warming {
  color: rgba(255, 255, 255, 0.3);
  animation: spin 1s linear infinite;
}

@keyframes pulse-ring {
  0%   { box-shadow: 0 0 0 0 rgba(239, 68, 68, 0.4); }
  70%  { box-shadow: 0 0 0 8px rgba(239, 68, 68, 0); }
  100% { box-shadow: 0 0 0 0 rgba(239, 68, 68, 0); }
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.transcript-enter-active, .transcript-leave-active { transition: opacity 0.15s; }
.transcript-enter-from, .transcript-leave-to { opacity: 0; }

</style>
