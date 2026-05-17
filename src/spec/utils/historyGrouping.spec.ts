import { groupHistoryItems } from "../../utils/historyGrouping";
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
    created_at: input.created_at ?? input.updated_at,
    updated_at: input.updated_at,
    series_id: input.series_id ?? null,
    period_key: input.period_key ?? null,
  };
}

describe("groupHistoryItems", () => {
  it("groups only series with at least two resolved occurrences", () => {
    const result = groupHistoryItems([
      quest({ id: "a", title: "solo", status: "completed", updated_at: "2026-05-01T10:00:00Z" }),
      quest({ id: "b", title: "daily", status: "completed", series_id: "series-1", updated_at: "2026-05-02T10:00:00Z" }),
      quest({ id: "c", title: "daily", status: "abandoned", series_id: "series-1", updated_at: "2026-05-03T10:00:00Z" }),
      quest({ id: "d", title: "single series", status: "completed", series_id: "series-2", updated_at: "2026-05-04T10:00:00Z" }),
    ]);

    const labels = result.rows.map((q) =>
      result.groupChildrenById[q.id] ? `group:${q.title}` : `quest:${q.title}`,
    );
    expect(labels).toEqual(["quest:single series", "group:daily", "quest:solo"]);

    const groupRep = result.rows.find((q) => result.groupChildrenById[q.id]);
    expect(groupRep?.series_id).toBe("series-1");
  });

  it("sorts group children by latest history timestamp, representative is the newest", () => {
    const result = groupHistoryItems([
      quest({ id: "older", title: "daily", status: "completed", series_id: "series-1", completed_at: "2026-05-01T10:00:00Z", updated_at: "2026-05-04T10:00:00Z" }),
      quest({ id: "newer", title: "daily", status: "abandoned", series_id: "series-1", updated_at: "2026-05-03T10:00:00Z" }),
    ]);

    expect(result.rows).toHaveLength(1);
    const rep = result.rows[0];
    expect(rep.id).toBe("newer");
    const children = result.groupChildrenById[rep.id];
    expect(children.map((c) => c.id)).toEqual(["newer", "older"]);
  });

  it("passes through a single resolved occurrence for a series without grouping", () => {
    const result = groupHistoryItems([
      quest({ id: "only", title: "weekly", status: "completed", series_id: "series-solo", updated_at: "2026-05-05T10:00:00Z" }),
    ]);

    expect(result.rows).toHaveLength(1);
    expect(result.groupChildrenById["only"]).toBeUndefined();
  });

  it("groups series with mixed completed and abandoned children", () => {
    const result = groupHistoryItems([
      quest({ id: "c1", title: "workout", status: "completed", series_id: "series-x", updated_at: "2026-05-03T10:00:00Z" }),
      quest({ id: "a1", title: "workout", status: "abandoned", series_id: "series-x", updated_at: "2026-05-02T10:00:00Z" }),
    ]);

    expect(result.rows).toHaveLength(1);
    const children = result.groupChildrenById[result.rows[0].id];
    expect(children).toHaveLength(2);
    const statuses = children.map((c) => c.status);
    expect(statuses).toContain("completed");
    expect(statuses).toContain("abandoned");
  });
});
