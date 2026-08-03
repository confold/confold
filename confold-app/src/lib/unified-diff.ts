export type UnifiedDiffLineKind =
  | "file_header"
  | "hunk"
  | "addition"
  | "deletion"
  | "context"
  | "metadata";

export type UnifiedDiffLine = {
  readonly kind: UnifiedDiffLineKind;
  readonly text: string;
};

function classifyLine(text: string): UnifiedDiffLineKind {
  if (text.startsWith("--- ") || text.startsWith("+++ ")) return "file_header";
  if (text.startsWith("@@")) return "hunk";
  if (text.startsWith("+")) return "addition";
  if (text.startsWith("-")) return "deletion";
  if (text.startsWith("\\")) return "metadata";
  return "context";
}

export function parseUnifiedDiff(source: string): readonly UnifiedDiffLine[] {
  const lines = source.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines.map((text) => ({ kind: classifyLine(text), text }));
}
