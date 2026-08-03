import { describe, expect, it } from "vitest";
import { isSemanticPath } from "./semantic";

describe("isSemanticPath", () => {
  it.each(["guide.md", "GUIDE.MARKDOWN", "notes.txt", "manual.rst", "page.adoc", "page.asciidoc"])(
    "accepts supported prose path %s",
    (path) => {
      expect(isSemanticPath(path)).toBe(true);
    },
  );

  it.each(["config.json", "photo.png", "README", "archive.md.zip"])(
    "rejects unsupported path %s",
    (path) => {
      expect(isSemanticPath(path)).toBe(false);
    },
  );
});
