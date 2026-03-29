import { createRouter, createWebHashHistory } from "vue-router";
import FocusView from "../views/FocusView.vue";
import QuestsView from "../views/QuestsView.vue";
import HistoryView from "../views/HistoryView.vue";
import SettingsView from "../views/SettingsView.vue";
import AddDeviceView from "../views/AddDeviceView.vue";
import DeviceView from "../views/DeviceView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/main" },
    { path: "/main", component: FocusView },
    { path: "/quests", component: QuestsView },
    { path: "/history", component: HistoryView },
    { path: "/settings", component: SettingsView },
    { path: "/settings/add-device", component: AddDeviceView },
    { path: "/settings/device/:id", component: DeviceView },
  ],
});

export default router;
