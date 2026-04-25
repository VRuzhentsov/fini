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
  <div class="fixed bottom-0 left-0 right-0 z-50 bg-base-200 border-t border-base-content/10" style="padding-bottom: env(safe-area-inset-bottom)">
    <Transition name="transcript">
      <div v-if="recording" class="px-4 py-2 text-sm opacity-50 border-b border-base-content/10">
        {{ voice.transcript.value || "Listening…" }}
      </div>
    </Transition>

    <form class="flex items-center gap-1 px-3 py-2" @submit.prevent="onSubmit">
      <textarea
        ref="textarea"
        v-model="text"
        data-testid="chat-input"
        class="textarea textarea-ghost flex-1 resize-none overflow-y-auto text-base leading-normal py-2 min-h-0"
        placeholder="Write a message…"
        rows="1"
        :readonly="recording"
        @input="autoResize"
        @keydown="onKeydown"
      />
      <button
        type="submit"
        data-testid="chat-submit"
        class="btn btn-ghost btn-circle btn-sm shrink-0"
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
        class="btn btn-ghost btn-circle btn-sm shrink-0"
        :class="{ 'text-error': recording, 'opacity-40': voice.warming.value }"
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
.transcript-enter-active, .transcript-leave-active { transition: opacity 0.15s; }
.transcript-enter-from, .transcript-leave-to { opacity: 0; }
</style>
