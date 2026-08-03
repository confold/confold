import { describe, expect, it } from "vitest";
import { parseUnifiedDiff } from "./unified-diff";

describe("parseUnifiedDiff", () => {
  it("classifies headers, hunks, additions, deletions, and context without changing text", () => {
    const source = "--- left\n+++ result\n@@ -1 +1 @@\n-old\n+new\n same\n";

    expect(parseUnifiedDiff(source)).toEqual([
      { kind: "file_header", text: "--- left" },
      { kind: "file_header", text: "+++ result" },
      { kind: "hunk", text: "@@ -1 +1 @@" },
      { kind: "deletion", text: "-old" },
      { kind: "addition", text: "+new" },
      { kind: "context", text: " same" },
    ]);
  });

  it("does not treat no-newline metadata as document content", () => {
    expect(parseUnifiedDiff("\\ No newline at end of file\n")).toEqual([
      { kind: "metadata", text: "\\ No newline at end of file" },
    ]);
  });
});
