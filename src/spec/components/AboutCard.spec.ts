import { mount } from "@vue/test-utils";
import AboutCard from "../../components/settings/AboutCard.vue";

describe("AboutCard", () => {
  it("links the source code and the license", () => {
    const wrapper = mount(AboutCard, {
      props: {
        version: "1.2.3",
        sourceUrl: "https://example.test/fini",
        license: "AGPL-3.0-or-later",
        licenseUrl: "https://example.test/fini/LICENSE",
      },
    });

    expect(wrapper.text()).toContain("1.2.3");
    expect(wrapper.find('a[href="https://example.test/fini"]').exists()).toBe(true);
    const license = wrapper.find('[data-testid="about-license"]');
    expect(license.attributes("href")).toBe("https://example.test/fini/LICENSE");
    expect(license.text()).toContain("AGPL-3.0-or-later");
  });
});
