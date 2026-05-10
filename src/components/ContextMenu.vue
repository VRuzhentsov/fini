<script setup lang="ts">
import { computed, nextTick, onUnmounted, reactive, ref, watch } from "vue";
import { useContextMenu, type ContextMenuTrigger, type MenuItem } from "../composables/useContextMenu";

const { state, close } = useContextMenu();

const MAIN_MAX_W = 240;
const MAIN_MIN_W = 160;
const SUB_MIN_W = 180;
const EDGE_PAD = 8;
const NARROW_BREAKPOINT = 640;

interface Viewport {
  width: number;
  height: number;
  bodyTop: number;
  bodyLeft: number;
  bodyRight: number;
  bodyBottom: number;
  bottomInset: number;
}

const viewport = reactive<Viewport>({
  width: 0,
  height: 0,
  bodyTop: 0,
  bodyLeft: 0,
  bodyRight: 0,
  bodyBottom: 0,
  bottomInset: 0,
});

function readSafeAreaBottom(): number {
  const v = getComputedStyle(document.documentElement).getPropertyValue("--safe-area-inset-bottom").trim();
  const n = parseFloat(v);
  return Number.isFinite(n) ? n : 0;
}

function measureViewport() {
  const w = window.innerWidth;
  const h = window.innerHeight;
  viewport.width = w;
  viewport.height = h;

  const main = (document.querySelector("main") ?? document.body) as HTMLElement;
  const r = main.getBoundingClientRect();
  viewport.bodyTop = Math.max(0, r.top);
  viewport.bodyLeft = Math.max(0, r.left);
  viewport.bodyRight = Math.min(w, r.right);
  viewport.bodyBottom = Math.min(h, r.bottom);

  const composer = document.querySelector(".chat-composer-bar") as HTMLElement | null;
  const composerH = composer ? composer.getBoundingClientRect().height : 0;
  viewport.bottomInset = composerH + readSafeAreaBottom() + EDGE_PAD;
}

interface Zone {
  side: "left" | "right";
  vertical: "top" | "bottom";
}

function classifyZone(trigger: ContextMenuTrigger): Zone {
  let x: number;
  let y: number;
  if (trigger.kind === "pointer") {
    x = trigger.x;
    y = trigger.y;
  } else {
    x = trigger.rect.left + trigger.rect.width / 2;
    y = trigger.rect.top + trigger.rect.height / 2;
  }
  const midX = (viewport.bodyLeft + viewport.bodyRight) / 2;
  const midY = (viewport.bodyTop + viewport.bodyBottom) / 2;
  return {
    side: x < midX ? "left" : "right",
    vertical: y < midY ? "top" : "bottom",
  };
}

const isNarrow = computed(() => viewport.width <= NARROW_BREAKPOINT);

const mainWidth = computed(() => {
  const aw = viewport.width;
  const cap = Math.min(MAIN_MAX_W, Math.floor(aw * 0.5));
  return Math.max(MAIN_MIN_W, cap);
});

const zone = computed<Zone | null>(() => (state.trigger ? classifyZone(state.trigger) : null));

const availableHeight = computed(() => {
  const top = viewport.bodyTop + EDGE_PAD;
  const bottom = viewport.height - viewport.bottomInset;
  return Math.max(120, bottom - top);
});

const mainStyle = computed<Record<string, string>>(() => {
  if (isNarrow.value) return {};
  if (!zone.value) return {};
  const w = mainWidth.value;
  const top = viewport.bodyTop + EDGE_PAD;
  const bottom = viewport.height - viewport.bottomInset;
  const style: Record<string, string> = {
    position: "fixed",
    width: `${w}px`,
    maxHeight: `${availableHeight.value}px`,
  };
  if (zone.value.side === "left") {
    style.left = `${viewport.bodyLeft + EDGE_PAD}px`;
  } else {
    style.right = `${Math.max(0, viewport.width - viewport.bodyRight) + EDGE_PAD}px`;
  }
  if (zone.value.vertical === "top") {
    style.top = `${top}px`;
  } else {
    style.bottom = `${viewport.height - bottom}px`;
  }
  return style;
});

const subWidth = computed(() => {
  if (!zone.value) return SUB_MIN_W;
  const bodyW = viewport.bodyRight - viewport.bodyLeft;
  const half = Math.max(0, bodyW / 2 - EDGE_PAD * 2);
  return Math.max(SUB_MIN_W, Math.floor(Math.min(half, MAIN_MAX_W * 1.4)));
});

const canFitSubmenuSideBySide = computed(() => {
  if (isNarrow.value) return false;
  const bodyW = viewport.bodyRight - viewport.bodyLeft;
  const totalNeeded = mainWidth.value + SUB_MIN_W + EDGE_PAD * 3;
  return bodyW >= totalNeeded;
});

const subStyle = computed<Record<string, string>>(() => {
  if (!zone.value || !canFitSubmenuSideBySide.value) return {};
  const w = subWidth.value;
  const top = viewport.bodyTop + EDGE_PAD;
  const bottom = viewport.height - viewport.bottomInset;
  const style: Record<string, string> = {
    position: "fixed",
    width: `${w}px`,
    maxHeight: `${availableHeight.value}px`,
  };
  if (zone.value.side === "left") {
    style.left = `${viewport.bodyLeft + EDGE_PAD + mainWidth.value}px`;
    style.borderTopLeftRadius = "0";
    style.borderBottomLeftRadius = "0";
    style.borderLeft = "0";
  } else {
    style.right = `${Math.max(0, viewport.width - viewport.bodyRight) + EDGE_PAD + mainWidth.value}px`;
    style.borderTopRightRadius = "0";
    style.borderBottomRightRadius = "0";
    style.borderRight = "0";
  }
  if (zone.value.vertical === "top") {
    style.top = `${top}px`;
  } else {
    style.bottom = `${viewport.height - bottom}px`;
  }
  return style;
});

const menuEl = ref<HTMLElement | null>(null);
const subEl = ref<HTMLElement | null>(null);
const hoveredParent = ref<string | null>(null);
const openedParent = ref<string | null>(null);
const overlayParent = ref<MenuItem | null>(null);

const overlayActive = computed(() => overlayParent.value !== null);

const HOVER_CLOSE_DELAY_MS = 300;
let hoverCloseTimer: ReturnType<typeof setTimeout> | null = null;

function cancelHoverClose() {
  if (hoverCloseTimer !== null) {
    clearTimeout(hoverCloseTimer);
    hoverCloseTimer = null;
  }
}

function scheduleHoverClose() {
  if (!canFitSubmenuSideBySide.value) return;
  cancelHoverClose();
  hoverCloseTimer = setTimeout(() => {
    hoveredParent.value = null;
    hoverCloseTimer = null;
  }, HOVER_CLOSE_DELAY_MS);
}

function clearSubmenuState() {
  cancelHoverClose();
  hoveredParent.value = null;
  openedParent.value = null;
  overlayParent.value = null;
}

function onItemClick(item: MenuItem) {
  if (item.disabled) return;
  if (item.children) {
    if (canFitSubmenuSideBySide.value) {
      cancelHoverClose();
      openedParent.value = openedParent.value === item.label ? null : item.label ?? null;
    } else {
      overlayParent.value = item;
    }
    return;
  }
  item.action?.();
  close();
}

function onChildClick(child: MenuItem) {
  if (child.disabled) return;
  child.action?.();
  close();
}

function onParentHover(item: MenuItem) {
  if (!item.children || item.disabled) return;
  if (canFitSubmenuSideBySide.value) {
    cancelHoverClose();
    hoveredParent.value = item.label ?? null;
  }
}

function clearHover() {
  scheduleHoverClose();
}

function onSubmenuEnter() {
  if (!visibleParent.value) return;
  cancelHoverClose();
  hoveredParent.value = visibleParent.value.label ?? null;
}

function backFromOverlay() {
  overlayParent.value = null;
}

function onPointerOutside(e: Event) {
  const t = e.target as Node | null;
  if (!t) return;
  if (menuEl.value?.contains(t)) return;
  if (subEl.value?.contains(t)) return;
  close();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    if (overlayActive.value) {
      backFromOverlay();
      return;
    }
    close();
  }
}

function onResize() {
  measureViewport();
}

watch(
  () => state.visible,
  async (v) => {
    if (v) {
      measureViewport();
      await nextTick();
      window.addEventListener("pointerdown", onPointerOutside, { capture: true });
      window.addEventListener("touchstart", onPointerOutside, { capture: true, passive: true });
      window.addEventListener("contextmenu", onPointerOutside, { capture: true });
      window.addEventListener("wheel", onPointerOutside, { capture: true, passive: true });
      window.addEventListener("keydown", onKeydown);
      window.addEventListener("scroll", close, { once: true, capture: true });
      window.addEventListener("resize", onResize);
    } else {
      window.removeEventListener("pointerdown", onPointerOutside, { capture: true });
      window.removeEventListener("touchstart", onPointerOutside, { capture: true });
      window.removeEventListener("contextmenu", onPointerOutside, { capture: true });
      window.removeEventListener("wheel", onPointerOutside, { capture: true });
      window.removeEventListener("keydown", onKeydown);
      window.removeEventListener("scroll", close, { capture: true });
      window.removeEventListener("resize", onResize);
      clearSubmenuState();
    }
  },
);

onUnmounted(() => {
  cancelHoverClose();
  window.removeEventListener("pointerdown", onPointerOutside, { capture: true });
  window.removeEventListener("touchstart", onPointerOutside, { capture: true });
  window.removeEventListener("contextmenu", onPointerOutside, { capture: true });
  window.removeEventListener("wheel", onPointerOutside, { capture: true });
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", close, { capture: true });
  window.removeEventListener("resize", onResize);
});

const visibleParent = computed<MenuItem | null>(() => {
  if (!canFitSubmenuSideBySide.value) return null;
  const lab = openedParent.value ?? hoveredParent.value;
  if (!lab) return null;
  return state.items.find((it) => it.label === lab && it.children) ?? null;
});

function isItemHot(item: MenuItem): boolean {
  return !!item.children && (hoveredParent.value === item.label || openedParent.value === item.label);
}
</script>

<template>
  <Teleport to="body">
    <template v-if="state.visible">
      <!-- Mobile bottom sheet (preserved) -->
      <div v-if="isNarrow && !overlayActive" class="ctx-scrim ctx-scrim-mobile" @click.self="close">
        <ul ref="menuEl" class="action-sheet mobile" @click.stop>
          <template v-for="(item, i) in state.items" :key="i">
            <li v-if="item.separator" class="sheet-separator" />
            <li v-else-if="item.children" class="sheet-submenu-host">
              <button
                class="sheet-item parent"
                :class="{ disabled: item.disabled }"
                @click.stop="onItemClick(item)"
              >
                <span>{{ item.label }}</span>
                <span class="sheet-chevron">›</span>
              </button>
            </li>
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: item.disabled, danger: item.label === 'Delete' }"
                @click.stop="onItemClick(item)"
              >{{ item.label }}</button>
            </li>
          </template>
        </ul>
      </div>

      <!-- Narrow / no-room: in-place overlay submenu with back -->
      <div v-else-if="overlayActive" class="ctx-scrim" @click.self="backFromOverlay">
        <ul ref="menuEl" class="action-sheet overlay">
          <li class="sheet-overlay-head">
            <button class="sheet-back" @click.stop="backFromOverlay">
              <span class="sheet-back-chev">‹</span>
              <span>Quest actions</span>
            </button>
            <span class="sheet-overlay-title">{{ overlayParent?.label }}</span>
          </li>
          <template v-for="(child, j) in overlayParent?.children ?? []" :key="j">
            <li v-if="child.separator" class="sheet-separator" />
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: child.disabled }"
                @click.stop="onChildClick(child)"
              >{{ child.label }}</button>
            </li>
          </template>
        </ul>
      </div>

      <!-- Wide side-sheet -->
      <template v-else>
        <ul ref="menuEl" class="action-sheet" :style="mainStyle">
          <template v-for="(item, i) in state.items" :key="i">
            <li v-if="item.separator" class="sheet-separator" />
            <li
              v-else-if="item.children"
              class="sheet-submenu-host"
              @mouseenter="onParentHover(item)"
              @mouseleave="clearHover"
            >
              <button
                class="sheet-item parent"
                :class="{ disabled: item.disabled, hot: isItemHot(item) }"
                @click.stop="onItemClick(item)"
              >
                <span>{{ item.label }}</span>
                <span class="sheet-chevron">›</span>
              </button>
            </li>
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: item.disabled, danger: item.label === 'Delete' }"
                @click.stop="onItemClick(item)"
              >{{ item.label }}</button>
            </li>
          </template>
        </ul>

        <ul
          v-if="visibleParent"
          ref="subEl"
          class="action-sheet sub"
          :class="{ 'sub-left': zone?.side === 'left', 'sub-right': zone?.side === 'right' }"
          :style="subStyle"
          data-testid="context-menu-submenu"
          @mouseenter="onSubmenuEnter"
          @mouseleave="clearHover"
        >
          <li class="sheet-sub-head">{{ visibleParent.label }}</li>
          <template v-for="(child, j) in visibleParent.children ?? []" :key="j">
            <li v-if="child.separator" class="sheet-separator" />
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: child.disabled }"
                @click.stop="onChildClick(child)"
              >{{ child.label }}</button>
            </li>
          </template>
        </ul>
      </template>
    </template>
  </Teleport>
</template>

<style scoped>
.action-sheet {
  z-index: 50;
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 0.375rem;
  margin: 0;
  overflow: auto;
  color: var(--fg-1);
  list-style: none;
  background: var(--color-base-100);
  border: 1px solid var(--color-border-soft);
  border-radius: 12px;
  box-shadow: var(--shadow-lg);
  scrollbar-width: thin;
}

.action-sheet.sub { z-index: 51; }

.sheet-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  width: 100%;
  min-height: 2.25rem;
  padding: 0.5rem 0.625rem;
  color: var(--fg-1);
  font: 500 0.875rem/1.2 Inter, Avenir, Helvetica, Arial, sans-serif;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 8px;
  cursor: pointer;
}

.sheet-item:hover:not(.disabled) { background: var(--color-base-200); }
.sheet-item.hot { background: var(--color-base-200); }
.sheet-item.danger { color: var(--color-error); }
.disabled { opacity: 0.35; pointer-events: none; }

.sheet-separator {
  height: 1px;
  margin: 0.25rem 0.375rem;
  background: var(--color-border-soft);
}

.sheet-submenu-host { position: relative; list-style: none; }
.sheet-chevron { color: var(--fg-4); font-size: 0.875rem; line-height: 1; }

.sheet-sub-head {
  padding: 0.375rem 0.625rem 0.5rem;
  margin: -0.125rem -0.125rem 0.25rem;
  color: var(--fg-2);
  font-size: 0.6875rem;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  border-bottom: 1px solid var(--color-border-soft);
}

.ctx-scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 0.5rem;
  background: rgba(0, 0, 0, 0.16);
}

.ctx-scrim-mobile {
  background: rgba(0, 0, 0, 0.24);
  backdrop-filter: blur(5px);
}

@media (prefers-color-scheme: dark) {
  .ctx-scrim { background: rgba(0, 0, 0, 0.45); }
  .ctx-scrim-mobile { background: rgba(0, 0, 0, 0.5); }
}

.action-sheet.overlay {
  width: min(20rem, calc(100vw - 1rem));
  max-height: calc(100vh - 1rem);
}

.sheet-overlay-head {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.625rem 0.5rem;
  margin: -0.125rem -0.125rem 0.25rem;
  border-bottom: 1px solid var(--color-border-soft);
}

.sheet-back {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0;
  color: var(--fg-2);
  font-size: 0.75rem;
  font-weight: 600;
  background: transparent;
  border: 0;
  cursor: pointer;
}

.sheet-back-chev { font-size: 1rem; line-height: 1; }

.sheet-overlay-title {
  margin-left: auto;
  color: var(--fg-2);
  font-size: 0.6875rem;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

/* Mobile bottom sheet — preserved behaviour */
.action-sheet.mobile {
  position: fixed;
  right: 0.75rem;
  bottom: max(0.75rem, env(safe-area-inset-bottom));
  left: 0.75rem;
  z-index: 50;
  width: auto;
  max-height: min(70vh, 32rem);
  padding: 0.5rem;
  border-radius: 18px;
}

.action-sheet.mobile::before {
  content: "";
  display: block;
  width: 2.25rem;
  height: 0.25rem;
  margin: 0.25rem auto 0.5rem;
  background: var(--fg-5);
  border-radius: 999px;
  opacity: 0.5;
}

.action-sheet.mobile .sheet-item {
  min-height: 3rem;
  font-size: 1rem;
}
</style>
