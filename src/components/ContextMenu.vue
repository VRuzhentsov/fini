<script setup lang="ts">
import { ref, watch, onUnmounted } from "vue";
import { useContextMenu, type MenuItem } from "../composables/useContextMenu";

const { state, close } = useContextMenu();

const menuEl = ref<HTMLElement | null>(null);
const hoveredParent = ref<string | null>(null);
const openedParent = ref<string | null>(null);

function onItemClick(item: MenuItem) {
  if (item.disabled || item.children) return;
  item.action?.();
  close();
}

function onChildClick(child: MenuItem) {
  if (child.disabled) return;
  child.action?.();
  close();
}

function toggleChildren(item: MenuItem) {
  if (item.disabled || !item.children) return;
  openedParent.value = openedParent.value === item.label ? null : item.label ?? null;
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
    openedParent.value = null;
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
      class="action-sheet"
      :style="{ top: `${state.y}px`, left: `${state.x}px` }"
    >
      <template v-for="(item, i) in state.items" :key="i">
        <li v-if="item.separator" class="sheet-separator" />
        <li
          v-else-if="item.children"
          class="sheet-submenu-host"
          @mouseenter="hoveredParent = item.label ?? null"
          @mouseleave="hoveredParent = null"
        >
          <button class="sheet-item parent" :class="{ disabled: item.disabled }" @click.stop="toggleChildren(item)">
            <span>{{ item.label }}</span>
            <span class="sheet-chevron">›</span>
          </button>
          <ul v-if="hoveredParent === item.label || openedParent === item.label" class="sheet-submenu">
            <li v-for="(child, j) in item.children" :key="j">
              <button
                class="sheet-item"
                :class="{ disabled: child.disabled }"
                @click.stop="onChildClick(child)"
              >{{ child.label }}</button>
            </li>
          </ul>
        </li>
        <li v-else>
          <button class="sheet-item" :class="{ disabled: item.disabled, danger: item.label === 'Delete' }" @click.stop="onItemClick(item)">{{ item.label }}</button>
        </li>
      </template>
    </ul>
  </Teleport>
</template>

<style scoped>
.action-sheet {
  position: fixed;
  z-index: 50;
  width: min(18rem, calc(100vw - 1.5rem));
  max-height: min(28rem, calc(100vh - 1.5rem));
  padding: 0.375rem;
  margin: 0;
  overflow: auto;
  color: var(--fg-1);
  list-style: none;
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
  box-shadow: var(--shadow-lg);
}

.sheet-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  min-height: 2.75rem;
  padding: 0.75rem 0.875rem;
  color: var(--fg-1);
  font: 500 0.9375rem/1.2 Inter, Avenir, Helvetica, Arial, sans-serif;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 10px;
}

.sheet-item { cursor: pointer; }
.sheet-item:hover:not(.disabled) { background: var(--color-base-200); }
.sheet-item.danger { color: var(--color-error); }
.disabled { opacity: 0.35; pointer-events: none; }
.sheet-separator { height: 1px; margin: 0.375rem 0.5rem; background: var(--color-border-soft); }
.sheet-submenu-host { position: relative; }
.sheet-chevron { color: var(--fg-4); font-size: 1rem; line-height: 1; }
.sheet-submenu {
  position: absolute;
  top: 0;
  left: calc(100% + 0.375rem);
  z-index: 51;
  width: 12rem;
  padding: 0.375rem;
  margin: 0;
  color: var(--fg-1);
  list-style: none;
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 14px;
  box-shadow: var(--shadow-lg);
}

@media (max-width: 640px) {
  .action-sheet {
    top: auto !important;
    left: 0.75rem !important;
    right: 0.75rem;
    bottom: max(0.75rem, env(safe-area-inset-bottom));
    width: auto;
    max-height: min(70vh, 32rem);
    padding: 0.5rem;
    border-radius: 18px;
  }

  .action-sheet::before {
    content: "";
    display: block;
    width: 2.25rem;
    height: 0.25rem;
    margin: 0.25rem auto 0.5rem;
    background: var(--fg-5);
    border-radius: 999px;
    opacity: 0.5;
  }

  .sheet-item { min-height: 3rem; font-size: 1rem; }

  .sheet-submenu {
    position: static;
    width: auto;
    margin: 0.25rem 0 0.375rem;
    box-shadow: none;
    background: var(--color-base-200);
  }

  .sheet-submenu .sheet-item {
    padding-left: 1.5rem;
  }
}
</style>
