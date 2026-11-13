// Test helpers only: nothing in the app imports this.
import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";

/**
 * `state` with its document (up to `upto`, by default all of it) parsed into the tree
 * that `syntaxTree(state)` returns.
 *
 * `ensureSyntaxTree` alone is not enough: it finishes the parse in the language's parse
 * context and returns that tree, but the tree stored in the state (what `syntaxTree`
 * reads) stays whatever `EditorState.create` or the last update managed in its 20 ms
 * budget. On a busy machine that is a partial tree. The empty update hands the finished
 * tree from the context to the state. Throws rather than return a partial tree, so a
 * test can't pass or fail on half a document.
 */
export function parsed(state: EditorState, upto = state.doc.length, timeout = 5000): EditorState {
  if (!ensureSyntaxTree(state, upto, timeout)) throw new Error(`document not parsed within ${timeout} ms`);
  const next = state.update({}).state;
  // Not `syntaxTreeAvailable`: it stays false while a fence names a language whose grammar has
  // not loaded (#180), though the markdown around the fence's code is parsed in full.
  if (syntaxTree(next).length < upto) throw new Error(`syntax tree covers ${syntaxTree(next).length} of ${upto} characters`);
  return next;
}
