import {
  parseChecklist,
  serializeChecklist,
  checklistCounts,
} from "../../utils/checklist";

describe("checklist", () => {
  it("round-trips parse/serialize", () => {
    const src = "- [ ] headphones <!--k=a1-->\n- [x] key fob <!--k=a2-->";
    const items = parseChecklist(src);
    expect(items).toEqual([
      { id: "a1", text: "headphones", checked: false },
      { id: "a2", text: "key fob", checked: true },
    ]);
    expect(serializeChecklist(items)).toBe(src);
  });

  it("returns an empty list for null/undefined/empty description", () => {
    expect(parseChecklist(null)).toEqual([]);
    expect(parseChecklist(undefined)).toEqual([]);
    expect(parseChecklist("")).toEqual([]);
  });

  it("ignores non-task-list lines", () => {
    const src = "some prose\n- [ ] headphones <!--k=a1-->\nmore prose";
    expect(parseChecklist(src)).toEqual([{ id: "a1", text: "headphones", checked: false }]);
  });

  it("computes done/total counts", () => {
    const src = "- [x] headphones <!--k=a1-->\n- [ ] key fob <!--k=a2-->\n- [x] lunch <!--k=a3-->";
    expect(checklistCounts(src)).toEqual([2, 3]);
  });

  it("computes zero/zero counts for a non-checklist description", () => {
    expect(checklistCounts(null)).toEqual([0, 0]);
  });
});
