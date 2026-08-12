import { Annotation, type Transaction, type TransactionSpec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

const EDITABLE_TEXT_SELECTOR = [
  'input:not([type])',
  'input[type="text"]',
  'input[type="search"]',
  'input[type="email"]',
  'input[type="url"]',
  'input[type="tel"]',
  'input[type="password"]',
  "textarea",
  '[contenteditable="true"]',
].join(",");

type NativeTextControl = HTMLInputElement | HTMLTextAreaElement;

interface NativeCompositionSnapshot {
  kind: "native";
  control: NativeTextControl;
  value: string;
  selectionStart: number;
  selectionEnd: number;
}

interface CodeMirrorCompositionSnapshot {
  kind: "codemirror";
  view: EditorView;
  doc: string;
  from: number;
  to: number;
}

type CompositionSnapshot = NativeCompositionSnapshot | CodeMirrorCompositionSnapshot;

export const codeMirrorTextInputSettled = Annotation.define<boolean>();
export const codeMirrorExternalTextInput = Annotation.define<boolean>();

interface CompositionSession {
  snapshot: CompositionSnapshot;
  predecessors: CompositionSession[];
  literalSpaceInput: boolean;
  postCompositionSpace: boolean;
  imeControlKey: boolean;
  switchStem: string | null;
  boundaryStem: string | null;
  ended: boolean;
  finalData: string | null;
  latestCompositionData: string | null;
  timer: ReturnType<typeof setTimeout> | null;
  editorSettlementQueued: boolean;
  editorSettlementPasses: number;
  corrected: boolean;
  lastCorrection: string | null;
  lastPlainInputValue: string | null;
  nativeInputSeen: boolean;
  syntheticInputSuppressed: boolean;
}

interface CodeMirrorInputController {
  session: CompositionSession;
  flush: () => void;
}

const pendingCodeMirrorInputs = new WeakMap<EditorView, CodeMirrorInputController>();

export function isCodeMirrorTextInputPending(view: EditorView) {
  return pendingCodeMirrorInputs.has(view);
}

export function flushCodeMirrorTextInput(view: EditorView) {
  pendingCodeMirrorInputs.get(view)?.flush();
}

const ASCII_COMPOSITION_SEGMENT = /^[A-Za-z0-9_]+$/;
const SHORT_ASCII_COMPOSITION_SEGMENT = /^[A-Za-z0-9_]{1,2}$/;
const IME_SPACING = /[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+/u;
const ONLY_IME_SPACING = /^[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+$/u;
const LEADING_IME_SPACING = /^[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+/u;
const TRAILING_IME_SPACING = /[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+$/u;
const SPACED_ASCII_PREFIX = /^[A-Za-z0-9_]+(?:[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+[A-Za-z0-9_]*)+/u;
const SINGLE_LETTER_ASCII_SPLIT = /^([A-Za-z0-9_]{3,})[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+([A-Za-z0-9_])$/u;
const SINGLE_LETTER_ASCII_SPLIT_PREFIX = /^([A-Za-z0-9_]{3,})[\u0020\u00a0\u2000-\u200a\u202f\u205f\u3000]+([A-Za-z0-9_])/u;

export function normalizeSpacedAsciiComposition(value: string, preserveSpaces = false) {
  if (preserveSpaces) return value;
  const segments = value.split(IME_SPACING);
  if (segments.length < 2) return value;
  const textSegments = segments.filter(Boolean);
  if (!textSegments.length || textSegments.some((segment) => !ASCII_COMPOSITION_SEGMENT.test(segment))) {
    return value;
  }
  const hasBoundarySpace = segments[0] === "" || segments[segments.length - 1] === "";
  const hasRepeatedShortGroups = textSegments.length >= 3
    && textSegments.every((segment) => SHORT_ASCII_COMPOSITION_SEGMENT.test(segment));
  if (hasBoundarySpace && !hasRepeatedShortGroups) {
    return value.replace(LEADING_IME_SPACING, "").replace(TRAILING_IME_SPACING, "");
  }
  if (!hasBoundarySpace && !hasRepeatedShortGroups) return value;
  return textSegments.join("");
}

function normalizeKnownImeArtifact(value: string, preserveSpaces: boolean, directSplitStem: string | null) {
  const normalized = normalizeSpacedAsciiComposition(value, preserveSpaces);
  if (normalized !== value || preserveSpaces || !directSplitStem) return normalized;
  const split = SINGLE_LETTER_ASCII_SPLIT.exec(value);
  return split?.[1] === directSplitStem ? `${split[1]}${split[2]}` : value;
}

function equivalentImePrefixLength(actual: string, expected: string) {
  let actualIndex = 0;
  let expectedIndex = 0;
  while (expectedIndex < expected.length) {
    const expectedCharacter = expected[expectedIndex]!;
    if (ONLY_IME_SPACING.test(expectedCharacter)) {
      while (expectedIndex < expected.length && ONLY_IME_SPACING.test(expected[expectedIndex]!)) {
        expectedIndex += 1;
      }
      if (!ONLY_IME_SPACING.test(actual[actualIndex] ?? "")) return null;
      while (actualIndex < actual.length && ONLY_IME_SPACING.test(actual[actualIndex]!)) {
        actualIndex += 1;
      }
    } else {
      if (actual[actualIndex] !== expectedCharacter) return null;
      actualIndex += 1;
      expectedIndex += 1;
    }
  }
  return actualIndex;
}

function directSplitCorrection(inserted: string, finalData: string | null, directSplitStem: string | null) {
  if (!directSplitStem || !finalData) return null;
  const split = SINGLE_LETTER_ASCII_SPLIT_PREFIX.exec(inserted);
  if (!split || split[1] !== directSplitStem) return null;
  const nextCharacter = split[2]!;
  const finalText = finalData.replace(LEADING_IME_SPACING, "").replace(TRAILING_IME_SPACING, "");
  const compactFinalText = finalText.replace(IME_SPACING, "");
  const compactArtifact = `${directSplitStem}${nextCharacter}`;
  if (finalText !== directSplitStem && finalText !== nextCharacter && compactFinalText !== compactArtifact) {
    return null;
  }
  return { length: split[0].length, normalized: compactArtifact };
}

function lateDirectSplitStem(session: CompositionSession, inserted: string) {
  if (!session.imeControlKey || session.literalSpaceInput || session.postCompositionSpace) return null;
  const split = SINGLE_LETTER_ASCII_SPLIT_PREFIX.exec(inserted);
  if (!split) return null;
  const finalText = session.finalData
    ?.replace(LEADING_IME_SPACING, "")
    .replace(TRAILING_IME_SPACING, "") ?? "";
  const compactFinalText = finalText.replace(IME_SPACING, "");
  return !finalText
    || finalText === split[1]
    || finalText === split[2]
    || compactFinalText === `${split[1]}${split[2]}`
    ? split[1]!
    : null;
}

function isDirectAsciiSplitForSession(session: CompositionSession, value: string) {
  const stem = session.switchStem ?? session.boundaryStem;
  const split = SINGLE_LETTER_ASCII_SPLIT.exec(value);
  return Boolean(session.switchStem && stem && split?.[1] === stem && !/\d/.test(split[2]!));
}

function ambiguousModeSwitchPayloadIsLiteral(session: CompositionSession, value: string) {
  const split = SINGLE_LETTER_ASCII_SPLIT.exec(value);
  if (!split || split[1] !== session.switchStem) return false;
  return /\d/.test(split[2]!);
}

export function configureTextInput(element: Element) {
  if (!element.matches(EDITABLE_TEXT_SELECTOR)) return;
  element.setAttribute("autocorrect", "off");
  element.setAttribute("autocapitalize", "none");
  element.setAttribute("spellcheck", "false");
  element.setAttribute("data-gramm", "false");
  if (!element.hasAttribute("autocomplete")) element.setAttribute("autocomplete", "off");
}

function configureTextInputsIn(node: Node) {
  if (!(node instanceof Element)) return;
  configureTextInput(node);
  node.querySelectorAll(EDITABLE_TEXT_SELECTOR).forEach(configureTextInput);
}

function nativeTextControl(target: EventTarget | null): NativeTextControl | null {
  if (target instanceof HTMLTextAreaElement) return target;
  if (!(target instanceof HTMLInputElement)) return null;
  return target.matches(EDITABLE_TEXT_SELECTOR) ? target : null;
}

function editableCompositionElement(target: EventTarget | null): Element | null {
  if (!(target instanceof Element)) return null;
  return target.closest(EDITABLE_TEXT_SELECTOR);
}

function createCompositionSnapshot(element: Element): CompositionSnapshot | null {
  const control = nativeTextControl(element);
  if (control) {
    return {
      kind: "native",
      control,
      value: control.value,
      selectionStart: control.selectionStart ?? control.value.length,
      selectionEnd: control.selectionEnd ?? control.value.length,
    };
  }
  if (!(element instanceof HTMLElement) || !element.isContentEditable) return null;
  const view = EditorView.findFromDOM(element);
  if (!view) return null;
  const selection = view.state.selection.main;
  return {
    kind: "codemirror",
    view,
    doc: view.state.doc.toString(),
    from: selection.from,
    to: selection.to,
  };
}

function compositionNormalization(
  inserted: string,
  finalData: string | null,
  preserveCompositionSpaces: boolean,
  preserveInsertedSpaces: boolean,
  directSplitStem: string | null,
) {
  if (finalData) {
    const equivalentPrefixLength = equivalentImePrefixLength(inserted, finalData);
    if (equivalentPrefixLength !== null) {
      const normalized = normalizeKnownImeArtifact(finalData, preserveCompositionSpaces, directSplitStem);
      if (normalized !== finalData) {
        return { length: equivalentPrefixLength, normalized };
      }
    }
    return preserveCompositionSpaces ? null : directSplitCorrection(inserted, finalData, directSplitStem);
  }
  const insertedCandidates = [inserted];
  const spacedPrefix = SPACED_ASCII_PREFIX.exec(inserted)?.[0];
  if (spacedPrefix && spacedPrefix !== inserted) insertedCandidates.push(spacedPrefix);
  for (const candidate of insertedCandidates) {
    const normalized = normalizeKnownImeArtifact(candidate, preserveInsertedSpaces, directSplitStem);
    if (normalized !== candidate) return { length: candidate.length, normalized };
  }
  return null;
}

function normalizeCompositionInControl(
  snapshot: NativeCompositionSnapshot,
  finalData: string | null,
  preserveCompositionSpaces = false,
  preserveInsertedSpaces = false,
  directSplitStem: string | null = null,
): string | null {
  const { control } = snapshot;
  const prefix = snapshot.value.slice(0, snapshot.selectionStart);
  const suffix = snapshot.value.slice(snapshot.selectionEnd);
  if (!control.value.startsWith(prefix) || !control.value.endsWith(suffix)) return null;

  const insertedEnd = suffix ? control.value.length - suffix.length : control.value.length;
  const inserted = control.value.slice(prefix.length, insertedEnd);
  const correction = compositionNormalization(
    inserted,
    finalData,
    preserveCompositionSpaces,
    preserveInsertedSpaces,
    directSplitStem,
  );
  if (!correction) return null;

  const changeFrom = prefix.length;
  const changeTo = changeFrom + correction.length;
  const selectionStart = control.selectionStart ?? insertedEnd;
  const selectionEnd = control.selectionEnd ?? selectionStart;
  const mapSelection = (position: number) => {
    if (position <= changeFrom) return position;
    if (position >= changeTo) return position + correction.normalized.length - correction.length;
    return changeFrom + correction.normalized.length;
  };
  control.value = `${control.value.slice(0, changeFrom)}${correction.normalized}${control.value.slice(changeTo)}`;
  if (correction.length === inserted.length) {
    const caret = changeFrom + correction.normalized.length;
    control.setSelectionRange(caret, caret);
  } else {
    control.setSelectionRange(mapSelection(selectionStart), mapSelection(selectionEnd));
  }
  return correction.normalized;
}

function codeMirrorCompositionCorrection(
  snapshot: CodeMirrorCompositionSnapshot,
  current: string,
  finalData: string | null,
  preserveCompositionSpaces = false,
  preserveInsertedSpaces = false,
  directSplitStem: string | null = null,
) {
  const { doc, from, to } = snapshot;
  const prefix = doc.slice(0, from);
  const suffix = doc.slice(to);
  if (!current.startsWith(prefix) || !current.endsWith(suffix)) return null;

  const insertedEnd = suffix ? current.length - suffix.length : current.length;
  const inserted = current.slice(prefix.length, insertedEnd);
  const correction = compositionNormalization(
    inserted,
    finalData,
    preserveCompositionSpaces,
    preserveInsertedSpaces,
    directSplitStem,
  );
  if (!correction) return null;
  return {
    from: prefix.length,
    to: prefix.length + correction.length,
    insert: correction.normalized,
  };
}

function normalizeCompositionInCodeMirror(
  snapshot: CodeMirrorCompositionSnapshot,
  finalData: string | null,
  preserveCompositionSpaces = false,
  preserveInsertedSpaces = false,
  directSplitStem: string | null = null,
): string | null {
  const { view } = snapshot;
  const correction = codeMirrorCompositionCorrection(
    snapshot,
    view.state.doc.toString(),
    finalData,
    preserveCompositionSpaces,
    preserveInsertedSpaces,
    directSplitStem,
  );
  if (!correction) return null;
  view.dispatch({
    changes: correction,
    userEvent: "input.type",
    filter: false,
  });
  return correction.insert;
}

function normalizationOptions(session: CompositionSession) {
  const finalDataHasBoundarySpace = Boolean(session.finalData)
    && (LEADING_IME_SPACING.test(session.finalData!) || TRAILING_IME_SPACING.test(session.finalData!));
  const preserveCompositionSpaces = session.literalSpaceInput
    || (session.postCompositionSpace && finalDataHasBoundarySpace);
  const preserveInsertedSpaces = preserveCompositionSpaces || session.postCompositionSpace;
  const finalText = session.finalData
    ?.replace(LEADING_IME_SPACING, "")
    .replace(TRAILING_IME_SPACING, "") ?? "";
  const finalDataConfirmsDirectSplit = Boolean(SINGLE_LETTER_ASCII_SPLIT.exec(finalText));
  let directSplitStem = preserveCompositionSpaces
    || (session.postCompositionSpace && !finalDataConfirmsDirectSplit)
    ? null
    : session.switchStem ?? session.boundaryStem;
  directSplitStem = lateDirectSplitStem(session, insertedText(session.snapshot) ?? "") ?? directSplitStem;
  return { preserveCompositionSpaces, preserveInsertedSpaces, directSplitStem };
}

function normalizeComposition(session: CompositionSession) {
  const { preserveCompositionSpaces, preserveInsertedSpaces, directSplitStem } = normalizationOptions(session);
  return session.snapshot.kind === "native"
    ? normalizeCompositionInControl(
      session.snapshot,
      session.finalData,
      preserveCompositionSpaces,
      preserveInsertedSpaces,
      directSplitStem,
    )
    : normalizeCompositionInCodeMirror(
      session.snapshot,
      session.finalData,
      preserveCompositionSpaces,
      preserveInsertedSpaces,
      directSplitStem,
    );
}

function nativeCompositionValueIsSettled(session: CompositionSession) {
  if (session.snapshot.kind !== "native" || !session.finalData) return false;
  const inserted = insertedText(session.snapshot);
  if (inserted === null) return false;
  return equivalentImePrefixLength(inserted, session.finalData) === inserted.length;
}

function asciiCompositionStem(value: string | null) {
  if (!value) return null;
  const trimmed = value.replace(LEADING_IME_SPACING, "").replace(TRAILING_IME_SPACING, "");
  if (ASCII_COMPOSITION_SEGMENT.test(trimmed)) return trimmed;
  return SINGLE_LETTER_ASCII_SPLIT.exec(trimmed)?.[1] ?? null;
}

function insertedText(snapshot: CompositionSnapshot) {
  if (snapshot.kind === "native") {
    const prefix = snapshot.value.slice(0, snapshot.selectionStart);
    const suffix = snapshot.value.slice(snapshot.selectionEnd);
    const current = snapshot.control.value;
    if (!current.startsWith(prefix) || !current.endsWith(suffix)) return null;
    return current.slice(prefix.length, suffix ? current.length - suffix.length : current.length);
  }
  const prefix = snapshot.doc.slice(0, snapshot.from);
  const suffix = snapshot.doc.slice(snapshot.to);
  const current = snapshot.view.state.doc.toString();
  if (!current.startsWith(prefix) || !current.endsWith(suffix)) return null;
  return current.slice(prefix.length, suffix ? current.length - suffix.length : current.length);
}

function stemObservedAtModeSwitch(session: CompositionSession) {
  return asciiCompositionStem(session.latestCompositionData)
    ?? asciiCompositionStem(insertedText(session.snapshot))
    ?? ((session.finalData?.length ?? 0) > 1 ? asciiCompositionStem(session.finalData) : null);
}

function codeMirrorCorrectionForSession(session: CompositionSession, current: string) {
  if (session.corrected || !session.ended || session.snapshot.kind !== "codemirror") return null;
  const { preserveCompositionSpaces, preserveInsertedSpaces, directSplitStem } = normalizationOptions(session);
  const { snapshot } = session;
  const prefix = snapshot.doc.slice(0, snapshot.from);
  const suffix = snapshot.doc.slice(snapshot.to);
  const insertedEnd = suffix && current.endsWith(suffix) ? current.length - suffix.length : current.length;
  const currentInserted = current.startsWith(prefix) ? current.slice(prefix.length, insertedEnd) : "";
  const effectiveDirectSplitStem = lateDirectSplitStem(session, currentInserted) ?? directSplitStem;
  return codeMirrorCompositionCorrection(
    snapshot,
    current,
    session.finalData,
    preserveCompositionSpaces,
    preserveInsertedSpaces,
    effectiveDirectSplitStem,
  );
}

function codeMirrorSessionIsWaitingForText(session: CompositionSession, current: string) {
  if (session.corrected || !session.ended || !session.finalData || session.snapshot.kind !== "codemirror") {
    return false;
  }
  const { preserveCompositionSpaces, directSplitStem } = normalizationOptions(session);
  const normalizedFinalData = normalizeKnownImeArtifact(
    session.finalData,
    preserveCompositionSpaces,
    directSplitStem,
  );
  const trimmedFinalData = session.finalData
    .replace(LEADING_IME_SPACING, "")
    .replace(TRAILING_IME_SPACING, "");
  const hasPotentialDirectSplit = Boolean(directSplitStem)
    && (trimmedFinalData === directSplitStem || ASCII_COMPOSITION_SEGMENT.test(trimmedFinalData));
  const waitingForTailOnlyCommit = session.imeControlKey
    && !session.literalSpaceInput
    && !session.postCompositionSpace
    && session.finalData.length === 1;
  if (normalizedFinalData === session.finalData && !hasPotentialDirectSplit && !waitingForTailOnlyCommit) return false;

  const { snapshot } = session;
  const prefix = snapshot.doc.slice(0, snapshot.from);
  const suffix = snapshot.doc.slice(snapshot.to);
  if (!current.startsWith(prefix) || !current.endsWith(suffix)) return false;
  const insertedEnd = suffix ? current.length - suffix.length : current.length;
  const inserted = current.slice(prefix.length, insertedEnd);
  if (waitingForTailOnlyCommit && !inserted) return true;
  const provisionalStem = session.imeControlKey
    && !session.literalSpaceInput
    && !session.postCompositionSpace
    && ASCII_COMPOSITION_SEGMENT.test(inserted)
    && inserted.length >= 3
    && session.finalData?.length === 1
    ? inserted
    : null;
  if (!inserted) return true;
  const compactInserted = inserted.replace(IME_SPACING, "");
  const compactFinalData = session.finalData.replace(IME_SPACING, "");
  if (compactFinalData.startsWith(compactInserted)) return true;
  if ((directSplitStem && inserted === directSplitStem) || provisionalStem) return true;
  const directSplit = SINGLE_LETTER_ASCII_SPLIT_PREFIX.exec(inserted);
  return Boolean(directSplitStem && directSplit?.[1] === directSplitStem);
}

/**
 * Makes a delayed CodeMirror DOM commit and its IME-boundary repair one atomic
 * transaction. The editor and its v-model never observe the transient spaced
 * document in between those two changes.
 */
export function filterCodeMirrorTextInputTransaction(
  view: EditorView | null,
  transaction: Transaction,
): Transaction | readonly TransactionSpec[] {
  if (!view || !transaction.docChanged || transaction.annotation(codeMirrorExternalTextInput)) return transaction;
  const controller = pendingCodeMirrorInputs.get(view);
  if (!controller) return transaction;

  let current = transaction.newDoc.toString();
  const corrections: TransactionSpec[] = [];
  for (const session of [...controller.session.predecessors, controller.session]) {
    const correction = codeMirrorCorrectionForSession(session, current);
    if (!correction) continue;
    corrections.push({ changes: correction, sequential: true });
    current = `${current.slice(0, correction.from)}${correction.insert}${current.slice(correction.to)}`;
    session.corrected = true;
    session.lastCorrection = correction.insert;
  }
  return corrections.length ? [transaction, ...corrections] : transaction;
}

/**
 * Applies one input policy to native controls and every CodeMirror editor.
 * It is installed before Vue mounts, so dynamically created fields and editors
 * receive the same IME composition correction without component-specific code.
 */
export function installTextInputProtection(root: HTMLElement) {
  configureTextInputsIn(root);
  const compositions = new WeakMap<Element, CompositionSession>();
  const activeSessions = new Set<CompositionSession>();
  let active = true;
  let finalizeCodeMirrorComposition: (element: Element, session: CompositionSession) => void;

  const handleFocusIn = (event: FocusEvent) => {
    if (event.target instanceof Element) configureTextInput(event.target);
  };
  root.addEventListener("focusin", handleFocusIn, true);

  const handleCompositionStart = (event: CompositionEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element) return;
    const previous = compositions.get(element);
    let predecessors: CompositionSession[] = [];
    if (previous) {
      // A new composition can start before CodeMirror's previous settlement
      // timer. Keep an unresolved boundary attached to the new session so a
      // late transaction from the previous composition is still repaired.
      if (previous.ended && previous.snapshot.kind === "codemirror") {
        const corrected = previous.corrected || normalizeComposition(previous) !== null;
        predecessors = previous.predecessors.slice();
        if (!corrected) predecessors.push(previous);
      }
      if (previous.timer !== null) clearTimeout(previous.timer);
      activeSessions.delete(previous);
    }
    const snapshot = createCompositionSnapshot(element);
    if (!snapshot) return;
    const session: CompositionSession = {
      snapshot,
      predecessors,
      literalSpaceInput: false,
      postCompositionSpace: false,
      imeControlKey: false,
      switchStem: null,
      boundaryStem: null,
      ended: false,
      finalData: null,
      latestCompositionData: null,
      timer: null,
      editorSettlementQueued: false,
      editorSettlementPasses: 0,
      corrected: false,
      lastCorrection: null,
      lastPlainInputValue: null,
      nativeInputSeen: false,
      syntheticInputSuppressed: false,
    };
    compositions.set(element, session);
    activeSessions.add(session);
    if (snapshot.kind === "codemirror") {
      pendingCodeMirrorInputs.set(snapshot.view, {
        session,
        flush: () => finalizeCodeMirrorComposition(element, session),
      });
    }
  };
  const handleKeyDown = (event: KeyboardEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element) return;
    const session = compositions.get(element);
    if (!session) return;
    const isSpace = event.key === " " || event.code === "Space";
    const isInputModeToggle = event.key === "CapsLock" || event.code === "CapsLock";
    if (isInputModeToggle || (isSpace && (event.ctrlKey || event.metaKey || event.keyCode === 229))) {
      session.imeControlKey = true;
      session.switchStem ??= stemObservedAtModeSwitch(session);
    }
    if (!session.ended && isSpace && !event.ctrlKey && !event.metaKey && event.keyCode !== 229) {
      session.literalSpaceInput = true;
    }
    if (session.ended && isSpace && !event.ctrlKey && !event.metaKey && !event.isComposing && event.keyCode !== 229) {
      session.postCompositionSpace = true;
    }
  };
  const handleCompositionUpdate = (event: CompositionEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element || !event.data) return;
    const session = compositions.get(element);
    if (session && !session.ended) {
      session.latestCompositionData = event.data;
      if (TRAILING_IME_SPACING.test(event.data)) {
        session.boundaryStem = asciiCompositionStem(event.data);
      }
    }
  };
  const handleBeforeInput = (event: InputEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element || !event.data || !IME_SPACING.test(event.data)) return;
    const session = compositions.get(element);
    if (!session) return;
    if (event.inputType === "insertText" || event.inputType === "insertCompositionText") {
      if (session.ended) {
        if (ONLY_IME_SPACING.test(event.data)) session.postCompositionSpace = true;
      } else if (ONLY_IME_SPACING.test(event.data)
        || (normalizeSpacedAsciiComposition(event.data) === event.data
          && !isDirectAsciiSplitForSession(session, event.data))
        || ambiguousModeSwitchPayloadIsLiteral(session, event.data)) {
        session.literalSpaceInput = true;
      }
    }
  };
  const dispatchCorrectedInput = (control: NativeTextControl, data: string) => {
    control.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      data,
      inputType: "insertReplacementText",
    }));
  };
  const cleanupSession = (element: Element, session: CompositionSession) => {
    if (session.timer !== null) clearTimeout(session.timer);
    session.timer = null;
    session.editorSettlementQueued = false;
    activeSessions.delete(session);
    if (compositions.get(element) === session) compositions.delete(element);
    if (session.snapshot.kind === "codemirror") {
      const { view } = session.snapshot;
      const controller = pendingCodeMirrorInputs.get(view);
      if (controller?.session === session) pendingCodeMirrorInputs.delete(view);
      if (controller?.session === session && active && view.dom.isConnected) {
        view.dispatch({ annotations: codeMirrorTextInputSettled.of(true), filter: false });
      }
    }
  };
  const applyCorrection = (session: CompositionSession) => {
    let lastCorrection: string | null = null;
    for (const pendingSession of [...session.predecessors, session]) {
      if (pendingSession.corrected) continue;
      const normalized = normalizeComposition(pendingSession);
      if (normalized === null) continue;
      pendingSession.corrected = true;
      pendingSession.lastCorrection = normalized;
      lastCorrection = normalized;
    }
    return lastCorrection;
  };
  finalizeCodeMirrorComposition = (element, session) => {
    if (session.snapshot.kind !== "codemirror" || compositions.get(element) !== session) return;
    if (!session.ended) {
      session.ended = true;
      session.finalData = session.latestCompositionData;
    }
    applyCorrection(session);
    cleanupSession(element, session);
  };
  const scheduleNativeFallback = (element: Element, session: CompositionSession) => {
    if (session.timer !== null) clearTimeout(session.timer);
    session.timer = setTimeout(() => {
      session.timer = null;
      if (!active || compositions.get(element) !== session || session.snapshot.kind !== "native") return;
      applyCorrection(session);
      const { control } = session.snapshot;
      if ((session.corrected || session.syntheticInputSuppressed)
        && session.lastPlainInputValue !== control.value) {
        dispatchCorrectedInput(control, session.lastCorrection ?? control.value);
      }
      cleanupSession(element, session);
    }, 120);
  };
  const scheduleEditorSettlement = (element: Element, session: CompositionSession, delay = 0) => {
    if (session.timer !== null) clearTimeout(session.timer);
    session.timer = null;
    const settle = () => {
      session.editorSettlementQueued = false;
      session.timer = null;
      if (!active || compositions.get(element) !== session || session.snapshot.kind !== "codemirror") return;
      if (applyCorrection(session) !== null) {
        cleanupSession(element, session);
        return;
      }
      const current = session.snapshot.view.state.doc.toString();
      const currentInserted = insertedText(session.snapshot);
      if (!session.switchStem
        && session.imeControlKey
        && session.finalData?.length === 1
        && currentInserted
        && currentInserted.length >= 3
        && ASCII_COMPOSITION_SEGMENT.test(currentInserted)) {
        session.switchStem = currentInserted;
      }
      const waitingForFinalText = [...session.predecessors, session]
        .some((pendingSession) => codeMirrorSessionIsWaitingForText(pendingSession, current));
      const retryDelays = [60, 180, 300];
      const nextDelay = retryDelays[session.editorSettlementPasses];
      session.editorSettlementPasses += 1;
      if (waitingForFinalText && nextDelay !== undefined) scheduleEditorSettlement(element, session, nextDelay);
      else cleanupSession(element, session);
    };
    if (delay === 0) {
      if (session.editorSettlementQueued) return;
      session.editorSettlementQueued = true;
      queueMicrotask(() => queueMicrotask(settle));
    } else {
      session.timer = setTimeout(settle, delay);
    }
  };
  const finalizeNativeComposition = (element: Element, session: CompositionSession) => {
    if (session.snapshot.kind !== "native") return;
    applyCorrection(session);
    if (session.nativeInputSeen) {
      cleanupSession(element, session);
    } else {
      scheduleNativeFallback(element, session);
    }
  };
  const finalizeBlurredComposition = (element: Element, session: CompositionSession) => {
    const normalized = applyCorrection(session);
    if (session.snapshot.kind === "native" && normalized !== null) {
      dispatchCorrectedInput(session.snapshot.control, normalized);
    }
    cleanupSession(element, session);
  };
  const handleCompositionEnd = (event: CompositionEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element) return;
    const session = compositions.get(element);
    if (!session) return;
    session.ended = true;
    session.finalData = event.data || session.latestCompositionData;
    if (session.imeControlKey) session.switchStem ??= stemObservedAtModeSwitch(session);
    if (session.snapshot.kind === "native") {
      // The value is usually already final in native controls. Correct it in
      // capture phase so Vue's compositionend-generated input sees only the
      // corrected value, while retaining the session for a late WebKit input.
      finalizeNativeComposition(element, session);
    } else {
      scheduleEditorSettlement(element, session);
    }
  };
  const handleInput = (event: Event) => {
    const element = editableCompositionElement(event.target);
    if (!element) return;
    const session = compositions.get(element);
    if (!session) return;
    if (!(event instanceof InputEvent)) {
      if (session.snapshot.kind === "native") {
        const { control } = session.snapshot;
        if (session.ended && session.finalData && !session.corrected
          && !nativeCompositionValueIsSettled(session)) {
          // Vue synthesizes this event inside compositionend. When WebKit has
          // not published the final value yet, letting it through would expose
          // a stale intermediate model value before the real InputEvent.
          session.syntheticInputSuppressed = true;
          event.stopImmediatePropagation();
          return;
        }
        session.lastPlainInputValue = control.value;
      }
      return;
    }
    if (event.isComposing) {
      if (event.data && IME_SPACING.test(event.data)) {
        if (ONLY_IME_SPACING.test(event.data)
          || (normalizeSpacedAsciiComposition(event.data) === event.data
            && !isDirectAsciiSplitForSession(session, event.data))) {
          session.literalSpaceInput = true;
        }
      }
      return;
    }
    if (session.snapshot.kind === "codemirror") {
      session.editorSettlementPasses = 0;
      scheduleEditorSettlement(element, session);
      return;
    }
    session.nativeInputSeen = true;
    applyCorrection(session);
    if (session.ended) cleanupSession(element, session);
    // This listener is in capture phase, so all consumers of the browser's
    // original input event observe the corrected value.
  };
  const handleBlur = (event: FocusEvent) => {
    const element = editableCompositionElement(event.target);
    if (!element) return;
    const session = compositions.get(element);
    if (!session) return;
    if (session.snapshot.kind === "codemirror") {
      session.editorSettlementPasses = 1;
      scheduleEditorSettlement(element, session);
    } else {
      finalizeBlurredComposition(element, session);
    }
  };
  root.addEventListener("keydown", handleKeyDown, true);
  root.addEventListener("beforeinput", handleBeforeInput, true);
  root.addEventListener("compositionstart", handleCompositionStart, true);
  root.addEventListener("compositionupdate", handleCompositionUpdate, true);
  root.addEventListener("compositionend", handleCompositionEnd, true);
  root.addEventListener("input", handleInput, true);
  root.addEventListener("blur", handleBlur, true);

  const observer = new MutationObserver((records) => {
    for (const record of records) {
      record.addedNodes.forEach(configureTextInputsIn);
    }
  });
  observer.observe(root, { childList: true, subtree: true });

  return () => {
    active = false;
    for (const session of activeSessions) {
      if (session.timer !== null) clearTimeout(session.timer);
      if (session.snapshot.kind === "codemirror") pendingCodeMirrorInputs.delete(session.snapshot.view);
    }
    activeSessions.clear();
    root.removeEventListener("focusin", handleFocusIn, true);
    root.removeEventListener("keydown", handleKeyDown, true);
    root.removeEventListener("beforeinput", handleBeforeInput, true);
    root.removeEventListener("compositionstart", handleCompositionStart, true);
    root.removeEventListener("compositionupdate", handleCompositionUpdate, true);
    root.removeEventListener("compositionend", handleCompositionEnd, true);
    root.removeEventListener("input", handleInput, true);
    root.removeEventListener("blur", handleBlur, true);
    observer.disconnect();
  };
}
