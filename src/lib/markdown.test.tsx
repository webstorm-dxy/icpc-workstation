import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownView } from "./markdown";

describe("MarkdownView math", () => {
  it("renders inline and display formulas with KaTeX", () => {
    const { container } = render(
      <MarkdownView content={"村庄数 $N$，且 $1\\leq x \\leq N \\le 10^3$。\n\n$$\\sum_{i=1}^{n} a_i$$"} />,
    );
    const html = container.innerHTML;
    expect(html).toContain("katex");
    expect(container.querySelectorAll(".katex").length).toBeGreaterThanOrEqual(2);
    // Raw TeX must be consumed, not left as text.
    expect(container.textContent).not.toContain("\\leq");
  });
});
