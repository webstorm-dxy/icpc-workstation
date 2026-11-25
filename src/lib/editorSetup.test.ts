import { indentMore } from "@codemirror/commands";
import { getIndentUnit } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { describe, expect, it } from "vitest";
import { cppEditorBase } from "./editorSetup";

function createView(doc: string): EditorView {
  return new EditorView({
    state: EditorState.create({ doc, extensions: cppEditorBase() }),
    parent: document.body,
  });
}

describe("C++ editor indentation", () => {
  it("uses the same four-space unit as the template and snippets", () => {
    const state = EditorState.create({ doc: "", extensions: cppEditorBase() });
    expect(getIndentUnit(state)).toBe(4);
    expect(state.tabSize).toBe(4);
  });

  it("inserts four spaces when indenting a line", () => {
    const view = createView("int main() {\nreturn 0;\n}");
    // Put the cursor on the second line's start.
    view.dispatch({ selection: { anchor: "int main() {\n".length } });
    indentMore(view);
    expect(view.state.doc.toString()).toBe("int main() {\n    return 0;\n}");
    view.destroy();
  });

  it("keeps the starter template at the same width", () => {
    // Mirrors DEFAULT_CODE in ProblemPage: the template's body line is four spaces in.
    const template = "int main() {\n    ios::sync_with_stdio(false);\n}";
    const view = createView(template);
    expect(getIndentUnit(view.state)).toBe(4);
    view.destroy();
  });
});
