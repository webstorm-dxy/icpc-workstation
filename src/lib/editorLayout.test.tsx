import { render } from "@testing-library/react";
import type { EditorView } from "@codemirror/view";
import CodeMirror from "@uiw/react-codemirror";
import { oneDark } from "@codemirror/theme-one-dark";
import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import { cppEditorBase } from "./editorSetup";

/**
 * Regression guard for "the editor cannot scroll".
 *
 * `@uiw/react-codemirror` renders `<div class="cm-theme">` around `.cm-editor`;
 * if that wrapper has no definite height, the editor grows with the code and
 * `.editor-area { overflow: hidden }` clips it instead of producing a scrollbar.
 */
beforeAll(() => {
  const style = document.createElement("style");
  style.textContent = readFileSync("src/styles.css", "utf8");
  document.head.appendChild(style);
});

describe("editor layout", () => {
  it("sizes the CodeMirror wrapper inside .editor-area", () => {
    const { container } = render(
      <div className="editor-area">
        <CodeMirror
          value={"int main(){}"}
          extensions={cppEditorBase()}
          theme={oneDark}
          height="100%"
        />
      </div>,
    );

    const wrapper = container.querySelector('.editor-area > [class*="cm-theme"]');
    expect(wrapper).not.toBeNull();
    expect(getComputedStyle(wrapper as Element).height).toBe("100%");

    const editor = container.querySelector(".cm-editor");
    expect(editor).not.toBeNull();
    expect(getComputedStyle(editor as Element).height).toBe("100%");

    const scroller = container.querySelector(".cm-scroller");
    expect(scroller).not.toBeNull();
    expect(getComputedStyle(scroller as Element).overflow).toContain("auto");
  });

  it("bounds tooltips to the editor viewport", () => {
    const providers = (cppEditorBase() as unknown[]).flat(Infinity) as {
      value?: { position?: string; tooltipSpace?: (view: EditorView) => DOMRect };
    }[];
    const config = providers.find(
      (item) => typeof item?.value?.tooltipSpace === "function",
    );
    expect(config).toBeDefined();
    expect(config!.value!.position).toBe("absolute");

    const scroller = document.createElement("div");
    const rect = { left: 10, top: 20, right: 610, bottom: 420, width: 600, height: 400 };
    scroller.getBoundingClientRect = () => rect as DOMRect;
    const view = { scrollDOM: scroller } as unknown as EditorView;

    // Completion/hover popups are clamped, flipped and shrunk against this
    // rect instead of the whole window.
    expect(config!.value!.tooltipSpace!(view)).toBe(rect);
  });
});
