<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useQuestStore } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";
import ChatInput from "../ChatInput.vue";

const questStore = useQuestStore();
const spaceStore = useSpaceStore();

const LAST_SPACE_KEY = "lastSpaceId";

const selectedSpaceId = ref("1");

onMounted(async () => {
  if (!spaceStore.spaces.length) await spaceStore.fetchSpaces();
  const saved = localStorage.getItem(LAST_SPACE_KEY);
  if (saved && spaceStore.spaces.find((s) => s.id === saved)) {
    selectedSpaceId.value = saved;
    return;
  }
  selectedSpaceId.value = spaceStore.spaces.find((s) => s.id === "1")?.id ?? spaceStore.spaces[0]?.id ?? "1";
});

function onSpaceChange() {
  localStorage.setItem(LAST_SPACE_KEY, selectedSpaceId.value);
}

async function onSubmit(text: string) {
  await questStore.createQuest({ title: text, space_id: selectedSpaceId.value });
}
</script>

<template>
  <div>
    <div v-if="spaceStore.spaces.length > 1" class="px-3 pt-2">
      <select
        v-model="selectedSpaceId"
        class="select select-sm select-ghost w-auto"
        @change="onSpaceChange"
      >
        <option v-for="space in spaceStore.spaces" :key="space.id" :value="space.id">
          {{ space.name }}
        </option>
      </select>
    </div>
    <ChatInput @submit="onSubmit" />
  </div>
</template>
