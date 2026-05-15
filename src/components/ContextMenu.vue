<script setup lang="ts">
import { computed, nextTick, onUnmounted, reactive, ref, watch } from "vue";
import { useContextMenu, type MenuItem } from "../composables/useContextMenu";

const { state, close } = useContextMenu();

const MAIN_MAX_W = 240;
const MAIN_MIN_W = 160;
const EDGE_PAD = 8;
const NARROW_BREAKPOINT = 640;
const ROW_H_EST = 32;

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

const isNarrow = computed(() => viewport.width <= NARROW_BREAKPOINT);

const mainWidth = computed(() => {
  const aw = viewport.width;
  const cap = Math.min(MAIN_MAX_W, Math.floor(aw * 0.5));
  return Math.max(MAIN_MIN_W, cap);
});

const availableHeight = computed(() => {
  const top = viewport.bodyTop + EDGE_PAD;
  const bottom = viewport.height - viewport.bottomInset;
  return Math.max(120, bottom - top);
});

const menuMeasuredHeight = ref(0);

const mainStyle = computed<Record<string, string>>(() => {
  if (isNarrow.value) return {};
  if (!state.trigger) return {};

  const w = mainWidth.value;
  const bodyL = viewport.bodyLeft + EDGE_PAD;
  const bodyR = viewport.bodyRight - EDGE_PAD;
  const bodyT = viewport.bodyTop + EDGE_PAD;
  const bodyB = viewport.height - viewport.bottomInset;
  const estH = menuMeasuredHeight.value ||
    Math.min(state.items.filter(it => !it.separator).length * ROW_H_EST + 12, availableHeight.value);

  const style: Record<string, string> = {
    position: "fixed",
    width: `${w}px`,
    maxHeight: `${availableHeight.value}px`,
  };

  if (state.trigger.kind === "pointer") {
    let x = state.trigger.x;
    let y = state.trigger.y;
    if (x + w > bodyR) x = bodyR - w;
    if (x < bodyL) x = bodyL;
    if (y + estH > bodyB) y = Math.max(bodyT, bodyB - estH);
    if (y < bodyT) y = bodyT;
    style.left = `${x}px`;
    style.top = `${y}px`;
    style["--cm-origin"] = "top left";
  } else {
    const rect = state.trigger.rect;
    // Horizontal: left-align if room to the right, else right-align
    let x = (bodyR - rect.right >= w) ? rect.left : (rect.right - w);
    if (x + w > bodyR) x = bodyR - w;
    if (x < bodyL) x = bodyL;
    style.left = `${x}px`;
    // Vertical: below by default; flip above when no room below but ≥80px above
    if (rect.bottom + estH <= bodyB) {
      style.top = `${rect.bottom}px`;
      style["--cm-origin"] = "top left";
    } else if (rect.top - bodyT >= 80) {
      style.bottom = `${viewport.height - rect.top}px`;
      style["--cm-origin"] = "bottom left";
    } else {
      style.top = `${bodyT}px`;
      style["--cm-origin"] = "top left";
    }
  }

  return style;
});

// ── Accordion ─────────────────────────────────────────────────────────────────

const menuEl = ref<HTMLElement | null>(null);
const expandedParent = ref<string | null>(null);

function toggleAccordion(item: MenuItem) {
  if (!item.children || item.disabled) return;
  expandedParent.value = expandedParent.value === item.label ? null : (item.label ?? null);
}

function clearSubmenuState() {
  expandedParent.value = null;
}

function onItemClick(item: MenuItem) {
  if (item.disabled) return;
  if (item.children) {
    toggleAccordion(item);
    return;
  }
  item.action?.();
  close();
}

function onChildClick(child: MenuItem) {
  if (child.disabled) return;
  expandedParent.value = null;
  child.action?.();
  close();
}

// ── Mobile sheet drag-to-dismiss ───────────────────────────────────────────────

const sheetDragY = ref(0);
const sheetIsDragging = ref(false);

interface VelocitySample { t: number; y: number; }
let velSamples: VelocitySample[] = [];
let dragStartY = 0;

function onSheetPointerDown(e: PointerEvent) {
  const handle = (e.currentTarget as HTMLElement);
  handle.setPointerCapture(e.pointerId);
  dragStartY = e.clientY;
  sheetDragY.value = 0;
  sheetIsDragging.value = true;
  velSamples = [{ t: e.timeStamp, y: e.clientY }];
}

function onSheetPointerMove(e: PointerEvent) {
  if (!sheetIsDragging.value) return;
  const raw = e.clientY - dragStartY;
  // rubber-band: up to 30px upward, elastic
  if (raw < 0) {
    sheetDragY.value = raw * 0.25;
  } else {
    sheetDragY.value = raw;
  }
  velSamples.push({ t: e.timeStamp, y: e.clientY });
  if (velSamples.length > 5) velSamples.shift();
}

function onSheetPointerUp(e: PointerEvent) {
  if (!sheetIsDragging.value) return;
  sheetIsDragging.value = false;

  // Calculate velocity from last 2 samples (px/ms)
  let velocity = 0;
  if (velSamples.length >= 2) {
    const a = velSamples[velSamples.length - 2];
    const b = velSamples[velSamples.length - 1];
    const dt = b.t - a.t;
    velocity = dt > 0 ? (b.y - a.y) / dt : 0;
  }

  const dy = e.clientY - dragStartY;
  if (dy > 120 || velocity > 0.6) {
    close();
  } else {
    sheetDragY.value = 0;
  }
}

const sheetStyle = computed(() => {
  if (sheetDragY.value === 0) return {} as Record<string, string>;
  return { transform: `translateY(${sheetDragY.value}px)`, transition: "none" } as Record<string, string>;
});

// ── Opening animation ──────────────────────────────────────────────────────────

const isOpening = ref(false);

// ── Outside click / keyboard ───────────────────────────────────────────────────

function onPointerOutside(e: Event) {
  const t = e.target as Node | null;
  if (!t) return;
  if (menuEl.value?.contains(t)) return;
  close();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}

function onResize() {
  measureViewport();
}

watch(
  () => state.visible,
  async (v) => {
    if (v) {
      measureViewport();
      menuMeasuredHeight.value = 0;
      isOpening.value = true;
      await nextTick();
      menuMeasuredHeight.value = menuEl.value?.getBoundingClientRect().height ?? 0;
      sheetDragY.value = 0;
      window.addEventListener("pointerdown", onPointerOutside, { capture: true });
      window.addEventListener("touchstart", onPointerOutside, { capture: true, passive: true });
      window.addEventListener("contextmenu", onPointerOutside, { capture: true });
      window.addEventListener("wheel", onPointerOutside, { capture: true, passive: true });
      window.addEventListener("keydown", onKeydown);
      window.addEventListener("scroll", close, { once: true, capture: true });
      window.addEventListener("resize", onResize);
      // Remove opening class after animation ends
      setTimeout(() => { isOpening.value = false; }, 260);
    } else {
      window.removeEventListener("pointerdown", onPointerOutside, { capture: true });
      window.removeEventListener("touchstart", onPointerOutside, { capture: true });
      window.removeEventListener("contextmenu", onPointerOutside, { capture: true });
      window.removeEventListener("wheel", onPointerOutside, { capture: true });
      window.removeEventListener("keydown", onKeydown);
      window.removeEventListener("scroll", close, { capture: true });
      window.removeEventListener("resize", onResize);
      clearSubmenuState();
      sheetDragY.value = 0;
    }
  },
);

onUnmounted(() => {
  window.removeEventListener("pointerdown", onPointerOutside, { capture: true });
  window.removeEventListener("touchstart", onPointerOutside, { capture: true });
  window.removeEventListener("contextmenu", onPointerOutside, { capture: true });
  window.removeEventListener("wheel", onPointerOutside, { capture: true });
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", close, { capture: true });
  window.removeEventListener("resize", onResize);
});
</script>

<template>
  <Teleport to="body">
    <template v-if="state.visible">
      <!-- ── Mobile bottom sheet ≤ 640px ──────────────────────────── -->
      <template v-if="isNarrow">
        <div class="ctx-scrim ctx-scrim-mobile" @click.self="close" />
        <ul
          ref="menuEl"
          class="action-sheet mobile"
          :class="{ opening: isOpening }"
          :style="sheetStyle"
          data-testid="context-menu-sheet"
          @click.stop
        >
          <!-- drag handle -->
          <li
            class="sheet-handle-row"
            @pointerdown.stop="onSheetPointerDown"
            @pointermove.stop="onSheetPointerMove"
            @pointerup.stop="onSheetPointerUp"
          >
            <span class="sheet-handle" />
          </li>

          <template v-for="(item, i) in state.items" :key="i">
            <li v-if="item.separator" class="sheet-separator" />
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: item.disabled, danger: item.danger, 'has-children': !!item.children }"
                :aria-expanded="item.children ? expandedParent === item.label : undefined"
                :data-danger="item.danger || undefined"
                @click.stop="onItemClick(item)"
              >
                <span v-if="item.icon" class="si-ico">
                  <component :is="item.icon" />
                </span>
                <span class="si-lab">{{ item.label }}</span>
                <span v-if="item.children" class="si-tr">
                  <span v-if="item.value" class="si-val">{{ item.value }}</span>
                  <svg class="si-chev" :class="{ rotated: expandedParent === item.label }" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m8.25 4.5 7.5 7.5-7.5 7.5"/></svg>
                </span>
                <span v-else-if="item.loading" class="si-tr">
                  <svg class="si-spn" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="9" stroke-width="2.5" stroke="currentColor" stroke-opacity="0.25"/><path d="M21 12a9 9 0 0 0-9-9" stroke-width="2.5" stroke="currentColor" stroke-linecap="round"/></svg>
                </span>
                <span v-else-if="item.badge != null" class="si-tr">
                  <span class="si-badge">{{ item.badge }}</span>
                </span>
              </button>
              <!-- accordion children -->
              <div v-if="item.children" class="acc" :data-open="expandedParent === item.label">
                <div class="acc-inner">
                  <template v-for="(child, j) in item.children" :key="j">
                    <div v-if="child.separator" class="sheet-separator" />
                    <button
                      v-else
                      class="sheet-item child"
                      :class="{ disabled: child.disabled }"
                      @click.stop="onChildClick(child)"
                    >
                      <span v-if="child.spaceColor" class="si-dot" :style="{ background: child.spaceColor }" />
                      <span v-else-if="child.icon" class="si-ico">
                        <component :is="child.icon" />
                      </span>
                      <span class="si-lab">{{ child.label }}</span>
                      <span v-if="child.selected" class="si-tr">
                        <svg class="si-chk" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
                      </span>
                    </button>
                  </template>
                </div>
              </div>
            </li>
          </template>
        </ul>
      </template>

      <!-- ── Wide side-sheet > 640px ─────────────────────────────── -->
      <template v-else>
        <div class="ctx-scrim" @click.self="close" />
        <ul
          ref="menuEl"
          class="action-sheet"
          :class="{ opening: isOpening }"
          :style="mainStyle"
          data-testid="context-menu"
        >
          <template v-for="(item, i) in state.items" :key="i">
            <li v-if="item.separator" class="sheet-separator" />
            <li v-else>
              <button
                class="sheet-item"
                :class="{ disabled: item.disabled, danger: item.danger, 'has-children': !!item.children, hot: item.children && expandedParent === item.label }"
                :aria-expanded="item.children ? expandedParent === item.label : undefined"
                :data-danger="item.danger || undefined"
                @click.stop="onItemClick(item)"
              >
                <span v-if="item.icon" class="si-ico">
                  <component :is="item.icon" />
                </span>
                <span class="si-lab">{{ item.label }}</span>
                <span v-if="item.children" class="si-tr">
                  <span v-if="item.value" class="si-val">{{ item.value }}</span>
                  <svg class="si-chev" :class="{ rotated: expandedParent === item.label }" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m8.25 4.5 7.5 7.5-7.5 7.5"/></svg>
                </span>
                <span v-else-if="item.loading" class="si-tr">
                  <svg class="si-spn" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="9" stroke-width="2.5" stroke="currentColor" stroke-opacity="0.25"/><path d="M21 12a9 9 0 0 0-9-9" stroke-width="2.5" stroke="currentColor" stroke-linecap="round"/></svg>
                </span>
                <span v-else-if="item.badge != null" class="si-tr">
                  <span class="si-badge">{{ item.badge }}</span>
                </span>
              </button>
              <!-- accordion children -->
              <div v-if="item.children" class="acc" :data-open="expandedParent === item.label">
                <div class="acc-inner">
                  <template v-for="(child, j) in item.children" :key="j">
                    <div v-if="child.separator" class="sheet-separator" />
                    <button
                      v-else
                      class="sheet-item child"
                      :class="{ disabled: child.disabled }"
                      @click.stop="onChildClick(child)"
                    >
                      <span v-if="child.spaceColor" class="si-dot" :style="{ background: child.spaceColor }" />
                      <span v-else-if="child.icon" class="si-ico">
                        <component :is="child.icon" />
                      </span>
                      <span class="si-lab">{{ child.label }}</span>
                      <span v-if="child.selected" class="si-tr">
                        <svg class="si-chk" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
                      </span>
                    </button>
                  </template>
                </div>
              </div>
            </li>
          </template>
        </ul>
      </template>
    </template>
  </Teleport>
</template>

<style scoped>
/* ── Base surface ─────────────────────────────────────────────────────────── */
.action-sheet {
  z-index: 50;
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0.375rem;
  margin: 0;
  overflow: auto;
  color: var(--fg-1);
  list-style: none;
  background: var(--color-base-200);
  border: 1px solid var(--color-border-soft);
  border-radius: 12px;
  box-shadow: var(--shadow-lg);
  scrollbar-width: thin;
  scrollbar-color: var(--fg-5) transparent;
}

/* ── Opening animations ───────────────────────────────────────────────────── */
.action-sheet.opening {
  animation: m-open 160ms cubic-bezier(0.4, 0, 0.2, 1) both;
  transform-origin: var(--cm-origin, top left);
}

.action-sheet.mobile.opening {
  animation: s-open 260ms cubic-bezier(0.34, 1.4, 0.64, 1) both;
  transform-origin: bottom center;
}

@keyframes m-open {
  from { opacity: 0; transform: scale(0.96) translateY(-2px); }
  to   { opacity: 1; transform: scale(1) translateY(0); }
}

@keyframes s-open {
  from { transform: translateY(100%); }
  to   { transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
  .action-sheet.opening,
  .action-sheet.mobile.opening,
  .acc,
  .si-chev { animation: none !important; transition: none !important; }
}

/* ── Row primitive ────────────────────────────────────────────────────────── */
.sheet-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  width: 100%;
  min-height: 2rem;
  padding: 0.375rem 0.625rem;
  color: var(--fg-1);
  font: 500 0.8125rem/1.2 Inter, Avenir, Helvetica, Arial, sans-serif;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 8px;
  cursor: pointer;
  user-select: none;
}

.sheet-item:hover:not(.disabled) { background: var(--color-base-100); }
.sheet-item.hot { background: var(--color-base-100); }

/* aria-expanded parent highlight */
.sheet-item[aria-expanded="true"] { background: var(--color-base-100); }

/* danger — quiet at rest, tinted on hover */
.sheet-item.danger { color: var(--fg-1); }
.sheet-item.danger:hover:not(.disabled) {
  background: rgba(228, 0, 75, 0.10);
  color: var(--color-error);
}
.sheet-item.danger:hover:not(.disabled) .si-ico { color: var(--color-error); }

@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .sheet-item.danger:hover:not(.disabled) {
    background: rgba(228, 0, 75, 0.20);
  }
}
:root[data-theme="dark"] .sheet-item.danger:hover:not(.disabled) {
  background: rgba(228, 0, 75, 0.20);
}

.disabled { opacity: 0.35; pointer-events: none; }

/* ── Row slots ────────────────────────────────────────────────────────────── */
.si-ico {
  width: 1rem;
  height: 1rem;
  flex: 0 0 1rem;
  color: var(--fg-3);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.si-ico :deep(svg) {
  width: 1rem;
  height: 1rem;
  stroke: currentColor;
  fill: none;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
  display: block;
}

.sheet-item:hover:not(.disabled) .si-ico { color: inherit; }
.sheet-item[aria-expanded="true"] .si-ico { color: inherit; }

.si-lab {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.si-tr {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  color: var(--fg-3);
  font-size: 0.75rem;
}

.si-val {
  color: var(--fg-3);
  max-width: 10ch;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.si-badge {
  padding: 1px 6px;
  border: 1px solid var(--color-border-soft);
  border-radius: 999px;
  font-size: 0.625rem;
  line-height: 1.4;
  color: var(--fg-2);
  font-variant-numeric: tabular-nums;
}

.si-chev {
  width: 0.75rem;
  height: 0.75rem;
  color: var(--fg-4);
  transition: transform 200ms cubic-bezier(0.4, 0, 0.2, 1);
}

.si-chev.rotated { transform: rotate(90deg); }

.si-chk {
  width: 0.75rem;
  height: 0.75rem;
  color: var(--fg-1);
}

.si-spn {
  width: 0.75rem;
  height: 0.75rem;
  animation: cm-spin 0.9s linear infinite;
}

@keyframes cm-spin { to { transform: rotate(360deg); } }

.si-dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  flex: 0 0 7px;
  margin-left: 4px;
  margin-right: 1px;
}

/* ── Accordion ────────────────────────────────────────────────────────────── */
.acc {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 200ms cubic-bezier(0.4, 0, 0.2, 1);
  overflow: hidden;
}

.acc[data-open="true"] { grid-template-rows: 1fr; }

.acc-inner {
  overflow: hidden;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 2px 0 4px;
}

.sheet-item.child { padding-left: 1.5rem; }

/* ── Separator ────────────────────────────────────────────────────────────── */
.sheet-separator {
  height: 1px;
  margin: 0.3125rem 0.375rem;
  background: var(--color-border-soft);
}

/* ── Scrim ────────────────────────────────────────────────────────────────── */
.ctx-scrim {
  position: fixed;
  inset: 0;
  z-index: 49;
  background: rgba(0, 0, 0, 0.16);
}

.ctx-scrim-mobile { background: rgba(0, 0, 0, 0.32); }

@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .ctx-scrim { background: rgba(0, 0, 0, 0.45); }
  :root:not([data-theme="light"]) .ctx-scrim-mobile { background: rgba(0, 0, 0, 0.55); }
}
:root[data-theme="dark"] .ctx-scrim { background: rgba(0, 0, 0, 0.45); }
:root[data-theme="dark"] .ctx-scrim-mobile { background: rgba(0, 0, 0, 0.55); }

/* ── Mobile bottom sheet ──────────────────────────────────────────────────── */
.action-sheet.mobile {
  position: fixed;
  right: 0;
  bottom: 0;
  left: 0;
  z-index: 50;
  width: auto;
  max-height: 85dvh;
  padding: 0 0.375rem max(0.375rem, env(safe-area-inset-bottom));
  border-radius: 1rem 1rem 0 0;
  border-bottom: 0;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--fg-5) transparent;
}

.action-sheet.mobile::-webkit-scrollbar { width: 8px; }
.action-sheet.mobile::-webkit-scrollbar-track { background: transparent; }
.action-sheet.mobile::-webkit-scrollbar-thumb {
  background: var(--fg-5);
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: padding-box;
}
.action-sheet.mobile::-webkit-scrollbar-thumb:hover {
  background: var(--fg-4);
  border: 2px solid transparent;
  background-clip: padding-box;
}

/* drag handle row */
.sheet-handle-row {
  display: flex;
  justify-content: center;
  padding: 0.5rem 0 0.25rem;
  cursor: grab;
  touch-action: none;
  list-style: none;
}

.sheet-handle-row:active { cursor: grabbing; }

.sheet-handle {
  display: block;
  width: 2.25rem;
  height: 0.25rem;
  border-radius: 999px;
  background: var(--fg-5);
  opacity: 0.5;
}

/* mobile touch targets are larger */
.action-sheet.mobile .sheet-item {
  min-height: 3rem;
  font-size: 0.9375rem;
  padding: 0.625rem 0.75rem;
}

.action-sheet.mobile .si-ico {
  width: 1.125rem;
  height: 1.125rem;
  flex: 0 0 1.125rem;
}

.action-sheet.mobile .si-ico :deep(svg) {
  width: 1.125rem;
  height: 1.125rem;
}
</style>
