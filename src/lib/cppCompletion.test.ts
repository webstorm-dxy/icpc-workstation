import { describe, expect, it } from "vitest";
import { buildCppOptions } from "./cppCompletion";

describe("C++ completion data", () => {
  it("contains keywords, STL and snippets", () => {
    const options = buildCppOptions();
    expect(options.some((option) => option.label === "sort")).toBe(true);
    expect(options.some((option) => option.label === "priority_queue")).toBe(true);
    expect(options.some((option) => option.label === "dsu")).toBe(true);
    expect(options.some((option) => option.label === "fori")).toBe(true);
    expect(options.length).toBeGreaterThan(100);
  });
});
