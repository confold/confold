const SEMANTIC_EXTENSIONS = new Set(["md", "markdown", "txt", "rst", "adoc", "asciidoc"]);

export function isSemanticPath(path: string): boolean {
  const extension = path.toLowerCase().split(".").pop() ?? "";
  return SEMANTIC_EXTENSIONS.has(extension);
}
