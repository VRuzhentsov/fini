<script setup lang="ts">
import { onMounted } from "vue";
import { useQuestStore } from "../stores/quest";
import ActiveQuestPanel from "../components/MainView/ActiveQuestPanel.vue";
import NewQuestForm from "../components/MainView/NewQuestForm.vue";

const store = useQuestStore();

onMounted(() => store.fetchQuests());
</script>

<template>
  <div class="flex flex-col gap-8 px-4 pt-4">
    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <section>
      <ActiveQuestPanel v-if="store.activeQuest" :quest="store.activeQuest" />
      <p v-else class="text-sm opacity-40">No active quest.</p>
    </section>

    <section>
      <NewQuestForm />
    </section>
  </div>
</template>
