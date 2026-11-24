import { describe, expect, it } from "vitest";
import { automationStatusLabel, difficultyLabel, verdictColor, verdictLabel } from "./format";

describe("format helpers", () => {
  it("maps verdicts", () => {
    expect(verdictLabel("AC")).toBe("通过");
    expect(verdictColor("WA")).toBe("red");
  });

  it("labels difficulty bands", () => {
    expect(difficultyLabel(800)).toBe("入门");
    expect(difficultyLabel(2000)).toBe("省选-");
  });

  it("labels automation states", () => {
    expect(automationStatusLabel("opening")).toBe("打开中");
    expect(automationStatusLabel("ready")).toBe("已就绪");
    expect(automationStatusLabel(undefined)).toBe("进行中");
  });
});
