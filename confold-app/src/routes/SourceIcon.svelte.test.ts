import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";
import SourceIcon from "./SourceIcon.svelte";

describe("SourceIcon", () => {
  it("renders the local Nuxeo brand mark for the Nuxeo token", () => {
    const { container } = render(SourceIcon, { icon: "nuxeo" });
    const image = container.querySelector("img");
    if (!image) throw new Error("Nuxeo icon did not render its local image mark");
    expect(image.getAttribute("src")).toBe("/nuxeo-logo.svg");
    expect(image.getAttribute("alt")).toBe("");
  });

  it("keeps glyph icons for existing sources", () => {
    const { container } = render(SourceIcon, { icon: "🌐" });
    expect(container.textContent).toContain("🌐");
    expect(container.querySelector("img")).toBeNull();
  });
});
