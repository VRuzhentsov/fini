import { mount, VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import ContextMenu from "../../components/ContextMenu.vue";
import { useContextMenu, type MenuItem } from "../../composables/useContextMenu";

const { open, openFromRect, close } = useContextMenu();

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
      value: "Work",
      children: [
        { label: "Personal", spaceColor: "var(--space-color-personal)", action: () => {} },
        { label: "Family",   spaceColor: "var(--space-color-family)",   action: () => {} },
        { label: "Work",     spaceColor: "var(--space-color-work)", selected: true },
      ],
    },
    { separator: true },
    { label: "Delete", danger: true, action: () => {} },
  ];
}

function openWideMenu() {
  // Viewport will be set to 1280 wide, so this triggers the wide side-sheet
  openFromRect(fakeRect({ left: 800, top: 200, right: 816, bottom: 216, width: 16, height: 16 }), questMenuItems());
}

function openNarrowMenu() {
  openFromRect(fakeRect({ left: 100, top: 200, right: 116, bottom: 216, width: 16, height: 16 }), questMenuItems());
}

function parentRow(): HTMLButtonElement {
  const el = document.body.querySelector<HTMLButtonElement>(
    "[data-testid='context-menu'] .sheet-item[aria-expanded]," +
    "[data-testid='context-menu-sheet'] .sheet-item[aria-expanded]",
  );
  if (!el) throw new Error('"Move to space" parent row not found');
  return el;
}

// ── Wide: accordion ────────────────────────────────────────────────────────────

describe("ContextMenu — accordion submenu (wide)", () => {
  let wrapper: VueWrapper<unknown> | null = null;
  let mainEl: HTMLElement;
  let composerEl: HTMLElement;
  let originalInnerWidth: number;
  let originalInnerHeight: number;

  beforeEach(() => {
    close();

    originalInnerWidth = window.innerWidth;
    originalInnerHeight = window.innerHeight;
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: 1280 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });

    mainEl = document.createElement("main");
    mainEl.getBoundingClientRect = () =>
      fakeRect({ top: 0, left: 0, right: 1280, bottom: 800, width: 1280, height: 800 });
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
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: originalInnerWidth });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: originalInnerHeight });
  });

  it("renders the side-sheet (not mobile sheet) on a wide viewport", async () => {
    wrapper = mount(ContextMenu);
    openWideMenu();
    await flushUi();

    expect(document.body.querySelector("[data-testid='context-menu']")).not.toBeNull();
    expect(document.body.querySelector("[data-testid='context-menu-sheet']")).toBeNull();
  });

  it("parent row has aria-expanded=false initially", async () => {
    wrapper = mount(ContextMenu);
    openWideMenu();
    await flushUi();

    expect(parentRow().getAttribute("aria-expanded")).toBe("false");
  });

  it("clicking parent row expands accordion and sets aria-expanded=true", async () => {
    wrapper = mount(ContextMenu);
    openWideMenu();
    await flushUi();

    parentRow().click();
    await flushUi();

    expect(parentRow().getAttribute("aria-expanded")).toBe("true");
    // children are rendered
    const childLabels = Array.from(
      document.body.querySelectorAll("[data-testid='context-menu'] .sheet-item.child"),
    ).map((el) => el.textContent?.trim());
    expect(childLabels).toEqual(expect.arrayContaining(["Personal", "Family", "Work"]));
  });

  it("clicking a child collapses the accordion", async () => {
    const childAction = jest.fn();
    wrapper = mount(ContextMenu);
    openFromRect(
      fakeRect({ left: 800, top: 200, right: 816, bottom: 216, width: 16, height: 16 }),
      [{ label: "Move to space", children: [{ label: "Personal", action: childAction }] }],
    );
    await flushUi();

    parentRow().click();
    await flushUi();

    const child = document.body.querySelector<HTMLButtonElement>(
      "[data-testid='context-menu'] .sheet-item.child",
    );
    expect(child).not.toBeNull();
    child!.click();
    await flushUi();

    expect(childAction).toHaveBeenCalledTimes(1);
    // menu is closed after child click
    expect(document.body.querySelector("[data-testid='context-menu']")).toBeNull();
  });

  it("opening a second picker collapses the first (single-open)", async () => {
    const items: MenuItem[] = [
      { label: "Alpha", children: [{ label: "A1", action: () => {} }] },
      { label: "Beta",  children: [{ label: "B1", action: () => {} }] },
    ];
    wrapper = mount(ContextMenu);
    openFromRect(fakeRect({ left: 800, top: 200, right: 816, bottom: 216, width: 16, height: 16 }), items);
    await flushUi();

    const [alphaBtn, betaBtn] = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>("[data-testid='context-menu'] .sheet-item[aria-expanded]"),
    );

    alphaBtn.click();
    await flushUi();
    expect(alphaBtn.getAttribute("aria-expanded")).toBe("true");

    betaBtn.click();
    await flushUi();
    expect(alphaBtn.getAttribute("aria-expanded")).toBe("false");
    expect(betaBtn.getAttribute("aria-expanded")).toBe("true");
  });

  it("danger row carries data-danger attribute", async () => {
    wrapper = mount(ContextMenu);
    openWideMenu();
    await flushUi();

    const dangerRow = document.body.querySelector<HTMLButtonElement>(
      "[data-testid='context-menu'] [data-danger]",
    );
    expect(dangerRow).not.toBeNull();
    expect(dangerRow!.textContent?.trim()).toBe("Delete");
  });

  it("selected child gets a checkmark element", async () => {
    wrapper = mount(ContextMenu);
    openWideMenu();
    await flushUi();

    parentRow().click();
    await flushUi();

    // Work is selected; its button should contain .si-chk
    const children = Array.from(
      document.body.querySelectorAll("[data-testid='context-menu'] .sheet-item.child"),
    );
    const workRow = children.find((el) => el.textContent?.includes("Work"));
    expect(workRow).toBeDefined();
    expect(workRow!.querySelector(".si-chk")).not.toBeNull();
  });
});

// ── Narrow: accordion + mobile sheet ──────────────────────────────────────────

describe("ContextMenu — mobile bottom-sheet (narrow)", () => {
  let wrapper: VueWrapper<unknown> | null = null;
  let mainEl: HTMLElement;
  let composerEl: HTMLElement;
  let originalInnerWidth: number;
  let originalInnerHeight: number;

  beforeEach(() => {
    close();

    originalInnerWidth = window.innerWidth;
    originalInnerHeight = window.innerHeight;
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: 375 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 812 });

    mainEl = document.createElement("main");
    mainEl.getBoundingClientRect = () =>
      fakeRect({ top: 0, left: 0, right: 375, bottom: 812, width: 375, height: 812 });
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
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: originalInnerWidth });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: originalInnerHeight });
  });

  it("renders the mobile bottom-sheet on a narrow viewport", async () => {
    wrapper = mount(ContextMenu);
    openNarrowMenu();
    await flushUi();

    expect(document.body.querySelector("[data-testid='context-menu-sheet']")).not.toBeNull();
    expect(document.body.querySelector("[data-testid='context-menu']")).toBeNull();
  });

  it("drag handle element exists in the mobile sheet", async () => {
    wrapper = mount(ContextMenu);
    openNarrowMenu();
    await flushUi();

    expect(
      document.body.querySelector("[data-testid='context-menu-sheet'] .sheet-handle"),
    ).not.toBeNull();
  });

  it("accordion works in the mobile sheet", async () => {
    wrapper = mount(ContextMenu);
    openNarrowMenu();
    await flushUi();

    const parent = document.body.querySelector<HTMLButtonElement>(
      "[data-testid='context-menu-sheet'] .sheet-item[aria-expanded]",
    );
    expect(parent).not.toBeNull();
    parent!.click();
    await flushUi();

    expect(parent!.getAttribute("aria-expanded")).toBe("true");
    const children = document.body.querySelectorAll("[data-testid='context-menu-sheet'] .sheet-item.child");
    expect(children.length).toBeGreaterThan(0);
  });

  it("no overlay submenu rendered (accordion replaced it)", async () => {
    wrapper = mount(ContextMenu);
    openNarrowMenu();
    await flushUi();

    const parent = document.body.querySelector<HTMLButtonElement>(
      "[data-testid='context-menu-sheet'] .sheet-item[aria-expanded]",
    );
    parent!.click();
    await flushUi();

    // There should be no separate overlay element at all
    expect(document.body.querySelector(".action-sheet.overlay")).toBeNull();
  });
});

// ── Placement (wide) ──────────────────────────────────────────────────────────

describe("ContextMenu — cursor-anchored placement (wide)", () => {
  let wrapper: VueWrapper<unknown> | null = null;
  let mainEl: HTMLElement;
  let composerEl: HTMLElement;
  let originalInnerWidth: number;
  let originalInnerHeight: number;

  const W = 1280;
  const H = 800;
  const items = [{ label: "Action", action: () => {} }];

  beforeEach(() => {
    close();
    originalInnerWidth = window.innerWidth;
    originalInnerHeight = window.innerHeight;
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: W });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: H });

    mainEl = document.createElement("main");
    mainEl.getBoundingClientRect = () =>
      fakeRect({ top: 0, left: 0, right: W, bottom: H, width: W, height: H });
    document.body.appendChild(mainEl);

    composerEl = document.createElement("div");
    composerEl.className = "chat-composer-bar";
    composerEl.getBoundingClientRect = () => fakeRect({ height: 0 });
    document.body.appendChild(composerEl);
  });

  afterEach(async () => {
    close();
    await nextTick();
    wrapper?.unmount();
    wrapper = null;
    mainEl.remove();
    composerEl.remove();
    Object.defineProperty(window, "innerWidth",  { configurable: true, value: originalInnerWidth });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: originalInnerHeight });
  });

  it("pointer trigger at center positions menu top-left near cursor", async () => {
    wrapper = mount(ContextMenu);
    const e = new MouseEvent("contextmenu", { clientX: 400, clientY: 300, bubbles: true });
    open(e, items);
    await flushUi();

    const menu = document.body.querySelector<HTMLElement>("[data-testid='context-menu']");
    expect(menu).not.toBeNull();
    expect(menu!.style.left).toBe("400px");
    expect(menu!.style.top).toBe("300px");
  });

  it("pointer trigger near right edge shifts menu left to fit", async () => {
    wrapper = mount(ContextMenu);
    // trigger at x=1250 → menu width 240 would go to 1490 > 1272 (1280-8)
    const e = new MouseEvent("contextmenu", { clientX: 1250, clientY: 300, bubbles: true });
    open(e, items);
    await flushUi();

    const menu = document.body.querySelector<HTMLElement>("[data-testid='context-menu']");
    expect(menu).not.toBeNull();
    const left = parseFloat(menu!.style.left);
    // Right edge (left + 240) must be within body right - pad
    expect(left + 240).toBeLessThanOrEqual(1280 - 8 + 1); // ≤ bodyR
    // Should have shifted left from the cursor
    expect(left).toBeLessThan(1250);
  });

  it("element trigger left side: menu left-aligns with rect.left", async () => {
    wrapper = mount(ContextMenu);
    // Rect on the left: rect.right=100, bodyRight=1272. Room right (1272-100=1172) >= 240 → left-align
    openFromRect(fakeRect({ left: 50, top: 200, right: 100, bottom: 220, width: 50, height: 20 }), items);
    await flushUi();

    const menu = document.body.querySelector<HTMLElement>("[data-testid='context-menu']");
    expect(menu).not.toBeNull();
    expect(menu!.style.left).toBe("50px");
    expect(menu!.style.top).toBe("220px");
  });

  it("element trigger right side: menu right-aligns with rect.right", async () => {
    wrapper = mount(ContextMenu);
    // Rect on the right: rect.right=1260. Room right (1272-1260=12) < 240 → right-align: x = 1260-240=1020
    openFromRect(fakeRect({ left: 1210, top: 200, right: 1260, bottom: 220, width: 50, height: 20 }), items);
    await flushUi();

    const menu = document.body.querySelector<HTMLElement>("[data-testid='context-menu']");
    expect(menu).not.toBeNull();
    const left = parseFloat(menu!.style.left);
    // left + 240 should approximately equal rect.right (1260), clamped to bodyR
    expect(left + 240).toBeCloseTo(1260, -1);
  });
});
