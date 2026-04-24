import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

export interface Reminder {
  id: string;
  quest_id: string;
  kind: "relative" | "absolute";
  mm_offset: number | null;
  due_at_utc: string | null;
  created_at: string;
  scheduled_notification_id: string | null;
}

export interface CreateReminderInput {
  quest_id: string;
  kind: "relative" | "absolute";
  mm_offset?: number | null;
  due_at_utc?: string | null;
}

export interface UpdateReminderInput {
  kind?: "relative" | "absolute";
  mm_offset?: number | null;
  due_at_utc?: string | null;
}

export const useReminderStore = defineStore("reminder", () => {
  async function getReminders(questId: string): Promise<Reminder[]> {
    return invoke<Reminder[]>("get_reminders", { questId });
  }

  async function createReminder(input: CreateReminderInput): Promise<Reminder> {
    return invoke<Reminder>("create_reminder", { input });
  }

  async function updateReminder(id: string, input: UpdateReminderInput): Promise<Reminder> {
    return invoke<Reminder>("update_reminder", { id, input });
  }

  async function deleteReminder(id: string): Promise<void> {
    return invoke("delete_reminder", { id });
  }

  return { getReminders, createReminder, updateReminder, deleteReminder };
});
