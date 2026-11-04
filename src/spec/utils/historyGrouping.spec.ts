import { sortHistoryItems } from "../../utils/historyGrouping";
import type { Quest } from "../../stores/quest";

const DAILY = JSON.stringify({ preset: "daily" });

function quest(input: Partial<Quest> & Pick<Quest, "id" | "title" | "status" | "updated_at">): Quest {
  return {
    id: input.id,
    space_id: input.space_id ?? "1",
    title: input.title,
    description: input.description ?? null,
    status: input.status,
    energy: input.energy ?? "medium",
    priority: input.priority ?? "medium",
    due: input.due ?? null,
    due_time: input.due_time ?? null,
    repeat_rule: input.repeat_rule ?? null,
    completed_at: input.completed_at ?? null,
    order_rank: input.order_rank ?? 0,
    focus_enter_count: input.focus_enter_count ?? 0,
    created_at: input.created_at ?? input.updated_at,
    updated_at: input.updated_at,
    is_checklist: input.is_checklist ?? false,
  };
}

/** One closed day of the repeating quest "daily" (ADR-0012: one quest, a row per day). */
function day(due: string, input: Partial<Quest> & Pick<Quest, "status" | "updated_at">): Quest {
  return quest({ id: "daily", title: "daily", repeat_rule: DAILY, due, ...input });
}

describe("sortHistoryItems", () => {
  it("shows only the latest closed day of a repeating quest", () => {
    const result = sortHistoryItems([
      day("2026-05-01", { status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      day("2026-05-03", { status: "abandoned", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].due).toBe("2026-05-03");
  });

  it("prefers completed_at over updated_at when picking the latest", () => {
    const result = sortHistoryItems([
      day("2026-05-01", { status: "completed", completed_at: "2026-05-05T10:00:00Z", updated_at: "2026-05-01T10:00:00Z" }),
      day("2026-05-03", { status: "abandoned", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].due).toBe("2026-05-01");
  });

  it("passes through a single closed day of a repeating quest unchanged", () => {
    const result = sortHistoryItems([day("2026-05-05", { status: "completed", updated_at: "2026-05-05T10:00:00Z" })]);

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("daily");
  });

  it("keeps one-time quests as-is", () => {
    const result = sortHistoryItems([
      quest({ id: "a", title: "solo", status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "b", title: "solo2", status: "abandoned", updated_at: "2026-05-02T10:00:00Z" }),
    ]);

    expect(result.map((q) => q.id)).toEqual(["b", "a"]);
  });

  it("sorts all rows newest-first", () => {
    const result = sortHistoryItems([
      quest({ id: "a", title: "solo", status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      day("2026-05-04", { status: "completed", updated_at: "2026-05-04T10:00:00Z" }),
      day("2026-05-02", { status: "abandoned", updated_at: "2026-05-02T10:00:00Z" }),
      quest({ id: "b", title: "solo2", status: "completed", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result.map((q) => q.id)).toEqual(["daily", "b", "a"]);
  });
});
