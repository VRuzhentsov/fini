<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../stores/space";

const spaceStore = useSpaceStore();
const open = ref(false);

onMounted(async () => {
  if (!spaceStore.spaces.length) await spaceStore.fetchSpaces();
});

function select(id: string | null) {
  spaceStore.selectSpace(id);
  open.value = false;
}

function onClickOutside(e: MouseEvent) {
  const el = (e.target as HTMLElement).closest('.space-picker');
  if (!el) open.value = false;
}

onMounted(() => window.addEventListener('click', onClickOutside));
onUnmounted(() => window.removeEventListener('click', onClickOutside));

function selectedLabel(): string {
  if (!spaceStore.selectedSpaceId) return "All spaces";
  return spaceStore.spaces.find(s => s.id === spaceStore.selectedSpaceId)?.name ?? "All spaces";
}

function selectedClass(): string {
  return spaceStore.selectedSpaceId ? (SPACE_COLOR_CLASS[spaceStore.selectedSpaceId] ?? "") : "";
}
</script>

<template>
  <div class="space-picker dropdown dropdown-end" :class="{ 'dropdown-open': open }">
    <button class="btn btn-ghost btn-sm" :class="selectedClass()" @click.stop="open = !open">
      {{ selectedLabel() }} &#9662;
    </button>
    <ul v-if="open" class="dropdown-content menu bg-base-200 rounded-box shadow-lg z-50 w-44 p-2">
      <li>
        <a :class="{ active: !spaceStore.selectedSpaceId }" @click="select(null)">All spaces</a>
      </li>
      <li v-for="space in spaceStore.spaces" :key="space.id">
        <a
          :class="[SPACE_COLOR_CLASS[space.id] ?? '', { active: spaceStore.selectedSpaceId === space.id }]"
          @click="select(space.id)"
        >{{ space.name }}</a>
      </li>
    </ul>
  </div>
</template>
