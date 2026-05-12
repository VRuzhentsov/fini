import { mount, VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import ContextMenu from "../../components/ContextMenu.vue";
import { useContextMenu, type MenuItem } from "../../composables/useContextMenu";

const { openFromRect, close } = useContextMenu();

function fakeRect(partial: Partial<DOMRect>): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: 0,
    height: 0,
    toJSON: () => ({}),
    ...partial,
  } as DOMRect;
}

async function flushUi() {
  for (let i = 0; i < 4; i += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

function questMenuItems(): MenuItem[] {
  return [
    { label: "Complete", action: () => {} },
    {
      label: "Move to space",
      children: [
        { label: "Personal", action: () => {} },
        { label: "Family", action: () => {} },
      ],
    },
  ];
}

function openMenu() {
  openFromRect(fakeRect({ left: 100, top: 200, right: 116, bottom: 216, width: 16, height: 16 }), questMenuItems());
}

function parentRow(): HTMLButtonElement {
  const el = document.body.querySelector<HTMLButtonElement>(
    ".action-sheet.mobile .sheet-submenu-host .sheet-item.parent",
  );
  if (!el) throw new Error('"Move to space" parent row not found in mobile sheet');
  return el;
}

describe("ContextMenu — narrow overlay submenu", () => {
  let wrapper: VueWrapper<unknown> | null = null;
  let mainEl: HTMLElement;
  let composerEl: HTMLElement;
  let originalInnerWidth: number;
  let originalInnerHeight: number;

  beforeEach(() => {
    close();

    originalInnerWidth = window.innerWidth;
    originalInnerHeight = window.innerHeight;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 600 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });

    // The component reads geometry off <main> and the composer bar.
    mainEl = document.createElement("main");
    mainEl.getBoundingClientRect = () =>
      fakeRect({ top: 0, left: 0, right: 600, bottom: 800, width: 600, height: 800 });
    document.body.appendChild(mainEl);

    composerEl = document.createElement("div");
    composerEl.className = "chat-composer-bar";
    composerEl.getBoundingClientRect = () => fakeRect({ height: 56 });
    document.body.appendChild(composerEl);
  });

  afterEach(async () => {
    close();
    await nextTick();
    wrapper?.unmount();
    wrapper = null;
    mainEl.remove();
    composerEl.remove();
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalInnerWidth });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: originalInnerHeight });
  });

  it("shows the mobile bottom sheet for the main menu on a narrow window", async () => {
    wrapper = mount(ContextMenu);
    openMenu();
    await flushUi();

    expect(document.body.querySelector(".action-sheet.mobile")).not.toBeNull();
    expect(document.body.querySelector(".action-sheet.overlay")).toBeNull();
  });

  it("opens the submenu as an overlay pinned to the bottom inset, growing upward", async () => {
    wrapper = mount(ContextMenu);
    openMenu();
    await flushUi();

    expect(parentRow().textContent).toContain("Move to space");
    parentRow().click();
    await flushUi();

    const overlay = document.body.querySelector<HTMLElement>(".action-sheet.overlay");
    expect(overlay).not.toBeNull();
    expect(document.body.querySelector(".action-sheet.mobile")).toBeNull();

    const style = overlay!.getAttribute("style") ?? "";
    expect(style).toContain("position: fixed");
    // bottomInset = composer(56) + safe-area(0) + edge-pad(8) = 64
    expect(style).toContain("bottom: 64px");
    expect(style).toContain("left: 8px");
    expect(style).toContain("right: 8px");
    // available height = (innerHeight 800 - bottomInset 64) - (bodyTop 0 + edge-pad 8) = 728
    expect(style).toMatch(/max-height:\s*728px/);
    // the old centred-at-top behaviour (fixed 20rem width) is gone
    expect(style).not.toContain("width: min(");

    // back-nav header carries the parent menu title
    expect(document.body.querySelector(".action-sheet.overlay .sheet-overlay-head")).not.toBeNull();
    expect(document.body.querySelector(".action-sheet.overlay .sheet-overlay-title")?.textContent).toBe("Move to space");
    expect(document.body.querySelector(".action-sheet.overlay .sheet-back")?.textContent).toContain("Quest actions");

    // submenu children are rendered
    const childLabels = Array.from(document.body.querySelectorAll(".action-sheet.overlay .sheet-item")).map((el) =>
      el.textContent?.trim(),
    );
    expect(childLabels).toEqual(expect.arrayContaining(["Personal", "Family"]));
  });

  it("returns to the mobile bottom sheet when the overlay back affordance is used", async () => {
    wrapper = mount(ContextMenu);
    openMenu();
    await flushUi();
    parentRow().click();
    await flushUi();
    expect(document.body.querySelector(".action-sheet.overlay")).not.toBeNull();

    document.body.querySelector<HTMLButtonElement>(".action-sheet.overlay .sheet-back")!.click();
    await flushUi();

    expect(document.body.querySelector(".action-sheet.overlay")).toBeNull();
    expect(document.body.querySelector(".action-sheet.mobile")).not.toBeNull();
  });
});
