import type { Attachment } from "svelte/attachments";

/** Ctrl/Cmd+A and Escape on a note list or tree, for multi-select. */
export function selectionKeys(onselectall: () => void, onclear: () => void): Attachment<HTMLElement> {
  return (el) => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        onselectall();
      } else if (e.key === "Escape") {
        onclear();
      }
    };
    el.addEventListener("keydown", handler);
    return () => el.removeEventListener("keydown", handler);
  };
}

/** Whether a click or key press asks to toggle a note rather than open it. */
export function isToggle(e: MouseEvent | KeyboardEvent): boolean {
  return e.ctrlKey || e.metaKey;
}
