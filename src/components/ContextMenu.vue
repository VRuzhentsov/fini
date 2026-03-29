<script setup lang="ts">
import { ref, watch, onUnmounted } from "vue";
import { useContextMenu, type MenuItem } from "../composables/useContextMenu";

const { state, close } = useContextMenu();

const menuEl = ref<HTMLElement | null>(null);
const hoveredParent = ref<string | null>(null);

function onItemClick(item: MenuItem) {
  if (item.disabled || item.children) return;
  item.action?.();
  close();
}

function onClickOutside(e: MouseEvent) {
  if (menuEl.value && !menuEl.value.contains(e.target as Node)) close();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}

watch(() => state.visible, (v) => {
  if (v) {
    window.addEventListener("click", onClickOutside, { capture: true });
    window.addEventListener("keydown", onKeydown);
    window.addEventListener("scroll", close, { once: true, capture: true });
  } else {
    window.removeEventListener("click", onClickOutside, { capture: true });
    window.removeEventListener("keydown", onKeydown);
    window.removeEventListener("scroll", close, { capture: true });
    hoveredParent.value = null;
  }
});

onUnmounted(() => {
  window.removeEventListener("click", onClickOutside, { capture: true });
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", close, { capture: true });
});
</script>

<template>
  <Teleport to="body">
    <ul
      v-if="state.visible"
      ref="menuEl"
      class="menu bg-base-200 rounded-box shadow-lg w-48 fixed z-50 p-2"
      :style="{ top: `${state.y}px`, left: `${state.x}px` }"
    >
      <template v-for="(item, i) in state.items" :key="i">
        <hr v-if="item.separator" class="border-base-content/10 my-1" />
        <li v-else-if="item.children" class="relative" @mouseenter="hoveredParent = item.label ?? null" @mouseleave="hoveredParent = null">
          <a class="flex justify-between" :class="{ disabled: item.disabled }">
            {{ item.label }}
            <span class="text-xs opacity-50">&#9656;</span>
          </a>
          <ul v-if="hoveredParent === item.label" class="menu bg-base-200 rounded-box shadow-lg w-44 p-2 absolute top-0 z-50 right-0 translate-x-full">
            <li v-for="(child, j) in item.children" :key="j">
              <a :class="{ disabled: child.disabled }" @click.stop="onItemClick(child)">{{ child.label }}</a>
            </li>
          </ul>
        </li>
        <li v-else>
          <a :class="{ disabled: item.disabled }" @click.stop="onItemClick(item)">{{ item.label }}</a>
        </li>
      </template>
    </ul>
  </Teleport>
</template>
