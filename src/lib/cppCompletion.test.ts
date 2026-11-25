import { describe, expect, it } from "vitest";
import { EDITOR_INDENT, buildCppOptions } from "./cppCompletion";

describe("C++ completion data", () => {
  it("contains keywords, STL and snippets", () => {
    const options = buildCppOptions();
    expect(options.some((option) => option.label === "sort")).toBe(true);
    expect(options.some((option) => option.label === "priority_queue")).toBe(true);
    expect(options.some((option) => option.label === "dsu")).toBe(true);
    expect(options.some((option) => option.label === "fori")).toBe(true);
    expect(options.length).toBeGreaterThan(100);
  });

  it("keeps the editor indent unit in step with the snippets", () => {
    expect(EDITOR_INDENT).toBe("    ");
    const snippets = buildCppOptions().filter((option) => option.type === "snippet");
    expect(snippets.length).toBeGreaterThan(10);
    for (const snippet of snippets) {
      const text = typeof snippet.apply === "string" ? snippet.apply : "";
      // A tab would render two or four columns depending on the viewer and stop
      // matching the template; the editor inserts EDITOR_INDENT on Tab.
      expect(text).not.toContain("\t");
      for (const line of text.split("\n").slice(1)) {
        const indent = line.match(/^ */)?.[0].length ?? 0;
        expect(indent % EDITOR_INDENT.length).toBe(0);
      }
    }
    const fori = snippets.find((snippet) => snippet.label === "fori");
    expect(fori?.apply).toContain(`\n${EDITOR_INDENT}\n}`);
  });
});
