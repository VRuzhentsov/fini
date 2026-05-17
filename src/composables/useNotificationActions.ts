import { onMounted } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { type Options, onAction, onNotificationReceived } from "@tauri-apps/plugin-notification";
import { useQuestStore } from "../stores/quest";
import { useToast } from "./useToast";

interface ActionPerformedPayload {
  actionId: string;
  inputValue?: string;
  notification: Options;
}

interface ForegroundFirePayload {
  questId: string;
  reminderId: string;
  body: string;
}

interface TapPayload {
  questId: string;
}

export function useNotificationActions() {
  const questStore = useQuestStore();
  const toast = useToast();

  onMounted(() => {
    // Plugin types incorrectly say callback receives Options; actual payload is ActionPerformedPayload
    onAction((raw) => {
      const payload = raw as unknown as ActionPerformedPayload;
      const reminderId = payload.notification?.extra?.["reminder_id"] as string | undefined;
      if (!reminderId) return;
      void invoke("notification_action", { actionId: payload.actionId, reminderId });
    });

    onNotificationReceived((notification) => {
      const reminderId = notification?.extra?.["reminder_id"] as string | undefined;
      if (!reminderId) return;
      void invoke("notification_tap", { reminderId });
    });

    void listen<ForegroundFirePayload>("notification://foreground-fire", (event) => {
      void questStore.setFocusQuest(event.payload.questId);
      toast.show(event.payload.body, "info", 2500);
    });

    void listen<TapPayload>("notification://tap", (event) => {
      void questStore.setFocusQuest(event.payload.questId);
    });
  });
}
