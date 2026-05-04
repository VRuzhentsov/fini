import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { useToast } from "./useToast";

const ANDROID_USER_AGENT_TOKEN = "Android";

function isAndroidWebView(): boolean {
  return typeof navigator !== "undefined" && navigator.userAgent.includes(ANDROID_USER_AGENT_TOKEN);
}

function needsReminderNotificationPermission(payload: { due: string | null }): boolean {
  return payload.due !== null;
}

export function useReminderNotifications() {
  const toast = useToast();

  async function ensureReminderNotificationsAllowed(payload: { due: string | null }): Promise<boolean> {
    if (!isAndroidWebView() || !needsReminderNotificationPermission(payload)) {
      return true;
    }

    try {
      if (await isPermissionGranted()) {
        return true;
      }

      const permission = await requestPermission();
      if (permission === "granted") {
        return true;
      }

      toast.error("Allow notifications to send Android reminder alerts.");
      return false;
    } catch (error) {
      toast.error("Could not request Android notification permission.");
      console.error("[reminder-notifications] permission request failed", error);
      return false;
    }
  }

  return { ensureReminderNotificationsAllowed };
}
