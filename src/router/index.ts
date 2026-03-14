import { createRouter, createWebHashHistory } from "vue-router";
import MainView from "../views/MainView.vue";
import QuestsView from "../views/QuestsView.vue";
import HistoryView from "../views/HistoryView.vue";
import SettingsView from "../views/SettingsView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/main" },
    { path: "/main", component: MainView },
    { path: "/quests", component: QuestsView },
    { path: "/history", component: HistoryView },
    { path: "/settings", component: SettingsView },
  ],
});

export default router;
