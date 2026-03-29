<script setup lang="ts">
import { useQuestStore, type Quest } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";
import { useContextMenu } from "../../composables/useContextMenu";

const props = defineProps<{ quest: Quest }>();
const store = useQuestStore();
const spaceStore = useSpaceStore();
const contextMenu = useContextMenu();

function onContextMenu(e: MouseEvent) {
  const moveItems = spaceStore.spaces
    .filter((s) => s.id !== props.quest.space_id)
    .map((s) => ({
      label: s.name,
      action: () => store.updateQuest(props.quest.id, { space_id: s.id }),
    }));
  contextMenu.open(e, [
    { label: "Complete", action: () => store.updateQuest(props.quest.id, { status: "completed" }) },
    { label: "Move to space", children: moveItems },
    { separator: true },
    { label: "Delete", action: () => store.deleteQuest(props.quest.id) },
  ]);
}

async function completeQuest() {
  await store.updateQuest(props.quest.id, { status: "completed" });
}

async function abandonQuest() {
  await store.updateQuest(props.quest.id, { status: "abandoned" });
}
</script>

<template>
  <div class="card card-bordered bg-base-200" @contextmenu="onContextMenu">
    <div class="card-body gap-3">
      <div class="flex items-start justify-between gap-4">
        <h2 class="card-title text-lg">{{ quest.title }}</h2>
        <div class="flex gap-2 shrink-0">
          <button class="btn btn-sm btn-ghost" @click="abandonQuest">Abandon</button>
          <button class="btn btn-sm btn-primary" @click="completeQuest">Complete</button>
        </div>
      </div>
      <p v-if="quest.description" class="text-sm opacity-70">{{ quest.description }}</p>
    </div>
  </div>
</template>
