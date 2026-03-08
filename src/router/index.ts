import { createRouter, createWebHashHistory } from "vue-router";
import QuestView from "../views/QuestView.vue";
import SettingsView from "../views/SettingsView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/quests" },
    { path: "/quests", component: QuestView },
    { path: "/settings", component: SettingsView },
  ],
});

export default router;
