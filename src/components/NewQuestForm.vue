<script setup lang="ts">
import { useQuestStore } from "../stores/quest";
import ChatInput from "./ChatInput.vue";

const store = useQuestStore();

async function onSubmit(text: string) {
  await store.createQuest({ title: text });
}
</script>

<template>
  <ChatInput @submit="onSubmit" />
</template>
