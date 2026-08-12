const FOCUSABLE_SELECTOR = [
  "button:not(:disabled)",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  "a[href]",
  '[tabindex]:not([tabindex="-1"])',
  '[contenteditable="true"]',
].join(",");

function visible(element: HTMLElement) {
  const style = getComputedStyle(element);
  return !element.hidden && style.display !== "none" && style.visibility !== "hidden";
}

function focusableElements(dialog: HTMLElement) {
  return Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(visible);
}

function topmostDialog(documentRoot: Document) {
  const dialogs = Array.from(documentRoot.querySelectorAll<HTMLElement>('[aria-modal="true"]')).filter(visible);
  return dialogs[dialogs.length - 1] ?? null;
}

export function installDialogAccessibility(documentRoot: Document) {
  let activeDialog: HTMLElement | null = null;
  let backgroundReturnTarget: HTMLElement | null = null;
  const lastFocusedInside = new WeakMap<HTMLElement, HTMLElement>();

  function focusDialog(dialog: HTMLElement) {
    const remembered = lastFocusedInside.get(dialog);
    const preferred = dialog.querySelector<HTMLElement>("[autofocus]")
      ?? focusableElements(dialog).find((element) => !element.classList.contains("icon-button"))
      ?? focusableElements(dialog)[0]
      ?? dialog;
    const target = remembered?.isConnected ? remembered : preferred;
    if (target === dialog && dialog.tabIndex < 0) dialog.tabIndex = -1;
    target.focus({ preventScroll: true });
  }

  function syncDialog() {
    const nextDialog = topmostDialog(documentRoot);
    if (nextDialog === activeDialog) return;
    const focused = documentRoot.activeElement instanceof HTMLElement ? documentRoot.activeElement : null;
    if (activeDialog && focused && activeDialog.contains(focused)) lastFocusedInside.set(activeDialog, focused);
    if (!activeDialog && nextDialog) backgroundReturnTarget = focused;
    activeDialog = nextDialog;
    queueMicrotask(() => {
      if (activeDialog) focusDialog(activeDialog);
      else if (backgroundReturnTarget?.isConnected) {
        backgroundReturnTarget.focus({ preventScroll: true });
        backgroundReturnTarget = null;
      }
    });
  }

  function trapFocus(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const dialog = topmostDialog(documentRoot);
    if (!dialog) return;
    const items = focusableElements(dialog);
    if (!items.length) {
      event.preventDefault();
      dialog.focus({ preventScroll: true });
      return;
    }
    const first = items[0]!;
    const last = items[items.length - 1]!;
    const focused = documentRoot.activeElement;
    if (event.shiftKey && (focused === first || !dialog.contains(focused))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (focused === last || !dialog.contains(focused))) {
      event.preventDefault();
      first.focus();
    }
  }

  const observer = new MutationObserver(syncDialog);
  observer.observe(documentRoot.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["style", "hidden", "aria-modal"] });
  documentRoot.addEventListener("keydown", trapFocus, true);
  syncDialog();

  return () => {
    observer.disconnect();
    documentRoot.removeEventListener("keydown", trapFocus, true);
  };
}
