import { mount } from "@vue/test-utils";
import { reactive } from "vue";
import SpacePicker from "../../components/SpacePicker.vue";
import { useSpaceStore } from "../../stores/space";

jest.mock("../../stores/space", () => ({
  useSpaceStore: jest.fn(),
  SPACE_COLOR_CLASS: {
    "1": "space-color-personal",
    "2": "space-color-family",
  },
}));

describe("SpacePicker", () => {
  let selectSpace: jest.Mock;
  let fetchSpaces: jest.Mock;
  let spaceStoreState: {
    selectedSpaceId: string | null;
    spaces: Array<{ id: string; name: string }>;
    selectSpace: jest.Mock;
    fetchSpaces: jest.Mock;
  };

  beforeEach(() => {
    selectSpace = jest.fn();
    fetchSpaces = jest.fn().mockResolvedValue(undefined);
    spaceStoreState = reactive({
      selectedSpaceId: null,
      spaces: [
        { id: "1", name: "Personal" },
        { id: "2", name: "Family" },
      ],
      selectSpace,
      fetchSpaces,
    });

    (useSpaceStore as unknown as jest.Mock).mockReturnValue(spaceStoreState);
  });

  it("updates the global space filter by default", async () => {
    const wrapper = mount(SpacePicker);

    await wrapper.find(".space-picker-all").trigger("click");
    await wrapper.find('[data-space-id="2"]').trigger("click");

    expect(selectSpace).toHaveBeenCalledWith("2");
  });

  it("emits local updates in controlled mode without mutating the global filter", async () => {
    const wrapper = mount(SpacePicker, {
      props: {
        modelValue: "1",
        allowAll: false,
      },
    });

    await wrapper.find(".space-chip-open").trigger("click");
    await wrapper.find('[data-space-id="2"]').trigger("click");

    expect(wrapper.emitted("update:modelValue")).toEqual([["2"]]);
    expect(selectSpace).not.toHaveBeenCalled();
    expect(wrapper.find(".space-chip-clear").exists()).toBe(false);
  });

  it("marks all picker controls as non-submit buttons", async () => {
    const wrapper = mount(SpacePicker, {
      props: {
        modelValue: "1",
        allowAll: true,
      },
    });

    await wrapper.find(".space-chip-open").trigger("click");

    expect(wrapper.find(".space-chip-open").attributes("type")).toBe("button");
    expect(wrapper.find(".space-chip-clear").attributes("type")).toBe("button");
    expect(wrapper.find('[data-space-id="2"]').attributes("type")).toBe("button");
    expect(wrapper.findAll(".space-menu-item")[2].attributes("type")).toBe("button");

    const allWrapper = mount(SpacePicker);
    expect(allWrapper.find(".space-picker-all").attributes("type")).toBe("button");
  });
});
