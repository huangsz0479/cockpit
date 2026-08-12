import { ref } from "vue";

export type ActionDialogKind = "confirm" | "prompt" | "notice";
export type ActionDialogTone = "default" | "danger" | "success" | "warning";

export interface ActionDialogState {
  id: number;
  kind: ActionDialogKind;
  tone: ActionDialogTone;
  title: string;
  message: string;
  detail?: string;
  confirmLabel: string;
  cancelLabel: string;
  inputLabel?: string;
  inputType?: "text" | "password";
  inputPlaceholder?: string;
  inputMinLength?: number;
  inputRequired?: boolean;
  inputValidationMessage?: string;
  trimInput?: boolean;
}

type ActionDialogRequest = Omit<ActionDialogState, "id" | "kind" | "tone" | "confirmLabel" | "cancelLabel"> & Partial<Pick<ActionDialogState,
  "tone" | "confirmLabel" | "cancelLabel"
>>;
type PromptDialogRequest = Omit<ActionDialogRequest, "inputType"> & Pick<ActionDialogState,
  "inputLabel" | "inputPlaceholder" | "inputMinLength" | "inputRequired" | "inputValidationMessage" | "trimInput"
> & { inputType?: "text" | "password" };

export function useActionDialog() {
  const actionDialog = ref<ActionDialogState | null>(null);
  let sequence = 0;
  let resolver: ((value: boolean | string | null) => void) | null = null;

  function open(state: Omit<ActionDialogState, "id">) {
    resolver?.(actionDialog.value?.kind === "prompt" ? null : false);
    actionDialog.value = { ...state, id: sequence += 1 };
    return new Promise<boolean | string | null>((resolve) => { resolver = resolve; });
  }

  async function confirmAction(request: ActionDialogRequest) {
    return await open({
      ...request,
      kind: "confirm",
      tone: request.tone ?? "default",
      confirmLabel: request.confirmLabel ?? "确认",
      cancelLabel: request.cancelLabel ?? "取消",
    }) === true;
  }

  async function promptAction(request: PromptDialogRequest) {
    const result = await open({
      ...request,
      kind: "prompt",
      tone: request.tone ?? "default",
      confirmLabel: request.confirmLabel ?? "继续",
      cancelLabel: request.cancelLabel ?? "取消",
      inputType: request.inputType ?? "text",
    });
    return typeof result === "string" ? result : null;
  }

  async function showNotice(request: ActionDialogRequest) {
    await open({
      ...request,
      kind: "notice",
      tone: request.tone ?? "default",
      confirmLabel: request.confirmLabel ?? "知道了",
      cancelLabel: "",
    });
  }

  function acceptActionDialog(value?: string) {
    const current = actionDialog.value;
    if (!current) return;
    const result = current.kind === "prompt"
      ? (current.trimInput === false ? value ?? "" : (value ?? "").trim())
      : true;
    actionDialog.value = null;
    const resolve = resolver;
    resolver = null;
    resolve?.(result);
  }

  function cancelActionDialog() {
    const current = actionDialog.value;
    if (!current) return;
    actionDialog.value = null;
    const resolve = resolver;
    resolver = null;
    resolve?.(current.kind === "prompt" ? null : false);
  }

  return {
    actionDialog,
    confirmAction,
    promptAction,
    showNotice,
    acceptActionDialog,
    cancelActionDialog,
  };
}
