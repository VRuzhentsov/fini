<script setup lang="ts">
import { useToast } from "../composables/useToast";
const { toasts } = useToast();
</script>

<template>
  <div class="toast-stack fixed left-4 right-4 z-[200] flex flex-col gap-2 pointer-events-none">
    <transition-group name="toast">
      <div
        v-for="t in toasts"
        :key="t.id"
        class="flex items-center gap-2 rounded-xl px-4 py-3 text-sm font-medium shadow-lg pointer-events-auto"
        :class="{
          'bg-red-600 text-white':   t.type === 'error',
          'bg-zinc-800 text-white':  t.type === 'info',
          'bg-green-600 text-white': t.type === 'success',
        }"
      >
        <span class="flex-1">{{ t.message }}</span>
        <button
          v-if="t.action"
          class="ml-2 rounded px-2 py-0.5 text-xs font-semibold opacity-90 ring-1 ring-white/40 hover:opacity-100"
          @click="t.action!.onClick()"
        >{{ t.action.label }}</button>
      </div>
    </transition-group>
  </div>
</template>

<style scoped>
.toast-stack {
  bottom: calc(var(--content-bottom-inset) + env(safe-area-inset-bottom) + 0.75rem);
}

.toast-enter-active, .toast-leave-active { transition: all 0.2s ease; }
.toast-enter-from { opacity: 0; transform: translateY(8px); }
.toast-leave-to   { opacity: 0; transform: translateY(-8px); }
</style>
