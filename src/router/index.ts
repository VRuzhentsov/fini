import { createRouter, createWebHashHistory } from "vue-router";
import FocusView from "../views/FocusView.vue";
import QuestsView from "../views/QuestsView.vue";
import HistoryView from "../views/HistoryView.vue";
import SettingsView from "../views/SettingsView.vue";
import DeviceView from "../views/DeviceView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/main" },
    { path: "/main", component: FocusView },
    { path: "/quests", component: QuestsView },
    { path: "/history", component: HistoryView },
    { path: "/settings", component: SettingsView },
    // Adding a device is a modal on the Settings page now, not a page of its
    // own: it is a short ceremony the user comes back from, not a place they
    // navigate to. The path stays as a way to *open* that modal, so the
    // Settings search entry and any existing deep link still work --
    // `DevicesSettingsSection` opens the dialog when this route is active.
    { path: "/settings/add-device", component: SettingsView },
    { path: "/settings/device/:id", component: DeviceView },
  ],
});

export default router;
