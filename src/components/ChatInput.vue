<script setup lang="ts">
import { ref, nextTick } from "vue";

const emit = defineEmits<{ submit: [text: string] }>();

const text = ref("");
const textarea = ref<HTMLTextAreaElement | null>(null);

function autoResize() {
  const el = textarea.value;
  if (!el) return;
  el.style.height = "auto";
  const lineHeight = parseInt(getComputedStyle(el).lineHeight);
  const maxHeight = lineHeight * 5;
  el.style.height = Math.min(el.scrollHeight, maxHeight) + "px";
}

function onSubmit() {
  const val = text.value.trim();
  if (!val) return;
  emit("submit", val);
  text.value = "";
  nextTick(autoResize);
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    onSubmit();
  }
}
</script>

<template>
  <div class="chat-composer-bar" style="padding-bottom: env(safe-area-inset-bottom)">
    <form class="chat-composer" @submit.prevent="onSubmit">
      <textarea
        ref="textarea"
        v-model="text"
        data-testid="chat-input"
        class="chat-composer-input textarea textarea-ghost"
        placeholder="Write a message…"
        rows="1"
        @input="autoResize"
        @keydown="onKeydown"
      />
      <button
        type="button"
        class="chat-composer-button"
        disabled
        aria-label="Attach"
        title="Attachments are not available yet"
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M18.375 12.739l-7.693 7.693a4.5 4.5 0 0 1-6.364-6.364l10.94-10.94A3 3 0 1 1 19.5 7.372L8.552 18.32m5.699-9.941l-7.81 7.81a1.5 1.5 0 0 0 2.112 2.13" />
        </svg>
      </button>
      <button
        v-if="text.trim()"
        type="submit"
        data-testid="chat-submit"
        class="chat-composer-button send"
        aria-label="Send"
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M6 12L3.269 3.125A59.769 59.769 0 0 1 21.485 12 59.768 59.768 0 0 1 3.27 20.875L5.999 12zm0 0h7.5" />
        </svg>
      </button>
      <button
        v-else
        type="button"
        class="chat-composer-button"
        disabled
        aria-label="Hold to speak"
        title="Voice input is not available yet"
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M12 18.75a6 6 0 0 0 6-6v-1.5m-6 7.5a6 6 0 0 1-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 0 1-3-3V4.5a3 3 0 1 1 6 0v8.25a3 3 0 0 1-3 3z" />
        </svg>
      </button>
    </form>
  </div>
</template>

<style scoped>
.chat-composer-bar {
  position: fixed;
  right: 0;
  bottom: 0;
  left: 0;
  z-index: 50;
  width: 100%;
  background: var(--color-page-bg);
  border-top: 1px solid var(--color-border-soft);
}

.chat-composer {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  width: calc(100% - 8px);
  margin: 4px;
  padding: 0.5rem 0.75rem;
  background: var(--color-base-200);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
}

.chat-composer-input {
  flex: 1;
  min-height: 0;
  max-height: calc(1.5em * 5 + 1rem);
  padding-top: 0.5rem;
  padding-bottom: 0.5rem;
  overflow-y: auto;
  color: var(--fg-1);
  font-size: 1rem;
  line-height: 1.5;
  resize: none;
  background: transparent;
  border: 0;
  outline: none;
  caret-color: var(--color-primary);
}

.chat-composer-input::placeholder {
  color: var(--fg-4);
  opacity: 1;
}

.chat-composer-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2.375rem;
  height: 2.375rem;
  flex-shrink: 0;
  padding: 0;
  align-self: center;
  color: var(--fg-3);
  cursor: pointer;
  background: transparent;
  border: 0;
  border-radius: 999px;
}

.chat-composer-button:hover:not(:disabled) {
  color: var(--fg-3);
  background: var(--color-base-300);
}

.chat-composer-button:disabled {
  cursor: not-allowed;
  opacity: 0.25;
}

.chat-composer-button.send {
  color: var(--color-primary-content);
  background: var(--color-primary);
}

.chat-composer-button.send:hover {
  color: var(--color-primary-content);
  filter: brightness(0.92);
}

.chat-composer-button svg {
  width: 1.25rem;
  height: 1.25rem;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
}
</style>
