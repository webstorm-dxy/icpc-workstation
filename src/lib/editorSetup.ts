/**
 * Base extensions for the C++ editor.
 *
 * Lives outside the page component so the layout contract can be tested:
 * - indentation matches the starter template and the completion snippets
 *   (CodeMirror defaults to two spaces, which made Tab-typed code drift);
 * - popups are bounded to the editor instead of the whole window, so the
 *   completion list cannot spill over the statement or test panels.
 */
import { cpp } from "@codemirror/lang-cpp";
import { indentUnit } from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { tooltips } from "@codemirror/view";
import { EDITOR_INDENT } from "./cppCompletion";

export function cppEditorBase(): Extension[] {
  return [
    cpp(),
    // Drives Tab, auto-indent on Enter and block re-indent.
    indentUnit.of(EDITOR_INDENT),
    // Keeps literal tabs (pasted code) at the same visual width.
    EditorState.tabSize.of(EDITOR_INDENT.length),
    // Popups are positioned against the whole window and rendered with
    // `position: fixed` by default, so completion/hover lists spill over the
    // statement and test panels (and, in WKWebView, Safari's fixed-position
    // probing can shift them). Rendering them inside the editor and bounding
    // their space to the editor's scroll viewport makes CodeMirror clamp, flip
    // and shrink them within the code area instead.
    tooltips({
      position: "absolute",
      tooltipSpace: (view) => view.scrollDOM.getBoundingClientRect(),
    }),
  ];
}
