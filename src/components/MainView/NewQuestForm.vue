<script setup lang="ts">
import { useQuestStore } from "../../stores/quest";
import { useSpaceStore } from "../../stores/space";
import ChatInput from "../ChatInput.vue";

const questStore = useQuestStore();
const spaceStore = useSpaceStore();

async function onSubmit(text: string) {
  const spaceId = spaceStore.selectedSpaceId ?? "1";
  await questStore.createQuest({ title: text, space_id: spaceId });
}
</script>

<template>
  <ChatInput @submit="onSubmit" />
</template>
