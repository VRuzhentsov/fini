<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useQuestStore, type Quest } from "../stores/quest";

const store = useQuestStore();
const history = computed(() =>
  store.quests.filter((q) => q.status === "completed" || q.status === "abandoned")
);

onMounted(() => store.fetchQuests());

// ── Context menu ──────────────────────────────────────────────────────────────

const menu = ref<{ x: number; y: number; quest: Quest } | null>(null);

function openMenu(e: MouseEvent, quest: Quest) {
  e.preventDefault();
  menu.value = { x: e.clientX, y: e.clientY, quest };
}

function closeMenu() {
  menu.value = null;
}

async function menuMakeActive() {
  if (!menu.value) return;
  await store.updateQuest(menu.value.quest.id, { status: "active" });
  closeMenu();
}

async function menuDelete() {
  if (!menu.value) return;
  await store.deleteQuest(menu.value.quest.id);
  closeMenu();
}

onMounted(() => window.addEventListener("click", closeMenu));
onUnmounted(() => window.removeEventListener("click", closeMenu));
</script>

<template>
  <div class="history-view">
    <h2>History</h2>

    <div v-if="store.error" class="error">{{ store.error }}</div>

    <p v-if="!history.length" class="empty">No completed or abandoned quests yet.</p>

    <ul v-else class="list">
      <li
        v-for="quest in history"
        :key="quest.id"
        class="item"
        @contextmenu="openMenu($event, quest)"
      >
        <span class="badge" :class="quest.status">{{ quest.status }}</span>
        <span class="title">{{ quest.title }}</span>
      </li>
    </ul>

    <!-- Context menu -->
    <Teleport to="body">
      <div
        v-if="menu"
        class="context-menu"
        :style="{ top: `${menu.y}px`, left: `${menu.x}px` }"
        @click.stop
      >
        <button @click="menuMakeActive">Make active</button>
        <hr />
        <button class="danger" @click="menuDelete">Delete</button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.history-view {
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

h2 {
  font-size: 1.25rem;
  font-weight: 600;
}

.error {
  color: red;
  font-size: 0.875rem;
}

.empty {
  opacity: 0.4;
  font-size: 0.875rem;
}

.list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.6rem 0.75rem;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.15);
  cursor: default;
  user-select: none;
}

.title {
  flex: 1;
  font-size: 0.95rem;
}

.badge {
  font-size: 0.75rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  text-transform: capitalize;
  flex-shrink: 0;
}

.badge.completed { background: #22c55e22; color: #22c55e; }
.badge.abandoned  { background: #f59e0b22; color: #f59e0b; }

.context-menu {
  position: fixed;
  z-index: 1000;
  min-width: 140px;
  background: var(--menu-bg, #fff);
  border: 1px solid rgba(128, 128, 128, 0.2);
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
  padding: 0.25rem;
  display: flex;
  flex-direction: column;
}

@media (prefers-color-scheme: dark) {
  .context-menu { --menu-bg: #1e1e1e; }
}

.context-menu button {
  background: none;
  border: none;
  padding: 0.5rem 0.75rem;
  text-align: left;
  cursor: pointer;
  border-radius: 5px;
  font: inherit;
  font-size: 0.875rem;
  color: inherit;
  width: 100%;
}

.context-menu button:hover {
  background: rgba(128, 128, 128, 0.12);
}

.context-menu button.danger {
  color: #e55;
}

.context-menu hr {
  border: none;
  border-top: 1px solid rgba(128, 128, 128, 0.15);
  margin: 0.25rem 0;
}
</style>
