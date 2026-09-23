import { createRouter, createWebHashHistory } from "vue-router";
import FocusView from "../views/FocusView.vue";
import QuestsView from "../views/QuestsView.vue";
import HistoryView from "../views/HistoryView.vue";
import SettingsView from "../views/settings/SettingsView.vue";
import DeviceView from "../views/settings/DeviceView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/main" },
    { path: "/main", component: FocusView },
    { path: "/quests", component: QuestsView },
    { path: "/history", component: HistoryView },
    { path: "/settings", component: SettingsView },
    // Adding a device has no route: it is a dialog on this page, a short
    // ceremony you come back from rather than a place you navigate to. A
    // path for it would be a URL that renders Settings and then opens
    // something on top -- a second way to describe one screen, and one the
    // back button would have to be taught about.
    { path: "/settings/device/:id", component: DeviceView },
  ],
});

export default router;
