import { createRouter, createWebHashHistory } from "vue-router";
import MainView from "../views/MainView.vue";
import SettingsView from "../views/SettingsView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/quests" },
    { path: "/quests", component: MainView },
    { path: "/settings", component: SettingsView },
  ],
});

export default router;
