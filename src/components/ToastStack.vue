<script setup lang="ts">
import { useToast } from "../composables/useToast";
const { toasts } = useToast();
</script>

<template>
  <div class="fixed bottom-20 left-4 right-4 z-[200] flex flex-col gap-2 pointer-events-none">
    <transition-group name="toast">
      <div
        v-for="t in toasts"
        :key="t.id"
        class="rounded-xl px-4 py-3 text-sm font-medium shadow-lg pointer-events-auto"
        :class="{
          'bg-red-600 text-white':   t.type === 'error',
          'bg-zinc-800 text-white':  t.type === 'info',
          'bg-green-600 text-white': t.type === 'success',
        }"
      >{{ t.message }}</div>
    </transition-group>
  </div>
</template>

<style scoped>
.toast-enter-active, .toast-leave-active { transition: all 0.2s ease; }
.toast-enter-from { opacity: 0; transform: translateY(8px); }
.toast-leave-to   { opacity: 0; transform: translateY(-8px); }
</style>
