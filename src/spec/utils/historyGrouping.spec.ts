import { sortHistoryItems } from "../../utils/historyGrouping";
import type { Quest } from "../../stores/quest";

function quest(input: Partial<Quest> & Pick<Quest, "id" | "title" | "status" | "updated_at">): Quest {
  return {
    id: input.id,
    space_id: input.space_id ?? "1",
    title: input.title,
    description: input.description ?? null,
    status: input.status,
    energy: input.energy ?? "medium",
    priority: input.priority ?? 1,
    pinned: input.pinned ?? false,
    due: input.due ?? null,
    due_time: input.due_time ?? null,
    repeat_rule: input.repeat_rule ?? null,
    completed_at: input.completed_at ?? null,
    order_rank: input.order_rank ?? 0,
    focus_enter_count: input.focus_enter_count ?? 0,
    created_at: input.created_at ?? input.updated_at,
    updated_at: input.updated_at,
    series_id: input.series_id ?? null,
    period_key: input.period_key ?? null,
  };
}

describe("sortHistoryItems", () => {
  it("shows only the latest resolved occurrence for a series", () => {
    const result = sortHistoryItems([
      quest({ id: "older", title: "daily", status: "completed", series_id: "series-1", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "newer", title: "daily", status: "abandoned", series_id: "series-1", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("newer");
  });

  it("prefers completed_at over updated_at when picking the latest", () => {
    const result = sortHistoryItems([
      quest({ id: "old-updated", title: "daily", status: "completed", series_id: "series-1", completed_at: "2026-05-05T10:00:00Z", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "new-updated", title: "daily", status: "abandoned", series_id: "series-1", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("old-updated");
  });

  it("passes through a single resolved occurrence for a series unchanged", () => {
    const result = sortHistoryItems([
      quest({ id: "only", title: "weekly", status: "completed", series_id: "series-solo", updated_at: "2026-05-05T10:00:00Z" }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("only");
  });

  it("keeps standalone quests (no series_id) as-is", () => {
    const result = sortHistoryItems([
      quest({ id: "a", title: "solo", status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "b", title: "solo2", status: "abandoned", updated_at: "2026-05-02T10:00:00Z" }),
    ]);

    expect(result.map((q) => q.id)).toEqual(["b", "a"]);
  });

  it("sorts all rows newest-first", () => {
    const result = sortHistoryItems([
      quest({ id: "a", title: "solo", status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "newer-series", title: "daily", status: "completed", series_id: "series-1", updated_at: "2026-05-04T10:00:00Z" }),
      quest({ id: "older-series", title: "daily", status: "abandoned", series_id: "series-1", updated_at: "2026-05-02T10:00:00Z" }),
      quest({ id: "b", title: "solo2", status: "completed", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result.map((q) => q.id)).toEqual(["newer-series", "b", "a"]);
  });
});
