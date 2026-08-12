<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { AlertTriangle, CheckCircle2, Eye, EyeOff, Info, ShieldAlert, X } from "lucide-vue-next";
import type { ActionDialogState } from "@/lib/actionDialog";

const props = defineProps<{ state: ActionDialogState }>();
const emit = defineEmits<{ confirm: [value?: string]; cancel: [] }>();
const input = ref("");
const dialogElement = ref<HTMLFormElement | null>(null);
const inputElement = ref<HTMLInputElement | null>(null);
const confirmButton = ref<HTMLButtonElement | null>(null);
const touched = ref(false);
const showPassword = ref(false);
const validationMessage = computed(() => {
  if (props.state.kind !== "prompt") return "";
  const value = props.state.trimInput === false ? input.value : input.value.trim();
  if (props.state.inputRequired && !value) return props.state.inputValidationMessage ?? "请输入内容";
  if (props.state.inputMinLength && value.length < props.state.inputMinLength) {
    return props.state.inputValidationMessage ?? `至少输入 ${props.state.inputMinLength} 个字符`;
  }
  return "";
});
const icon = computed(() => props.state.tone === "danger"
  ? ShieldAlert
  : props.state.tone === "warning"
    ? AlertTriangle
    : props.state.tone === "success"
      ? CheckCircle2
      : Info);

watch(() => props.state.id, async () => {
  input.value = "";
  touched.value = false;
  showPassword.value = false;
  await nextTick();
  if (props.state.kind === "prompt") inputElement.value?.focus();
  else confirmButton.value?.focus();
}, { immediate: true });

function submit() {
  touched.value = true;
  if (validationMessage.value) return;
  emit("confirm", props.state.kind === "prompt" ? input.value : undefined);
}

function trapFocus(event: KeyboardEvent) {
  const dialog = dialogElement.value;
  if (!dialog) return;
  const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(
    'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
  )).filter((element) => element.offsetParent !== null || document.activeElement === element);
  if (!focusable.length) return;
  const first = focusable[0]!;
  const last = focusable[focusable.length - 1]!;
  if (event.shiftKey && (document.activeElement === first || !dialog.contains(document.activeElement))) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (document.activeElement === last || !dialog.contains(document.activeElement))) {
    event.preventDefault();
    first.focus();
  }
}
</script>

<template>
  <div class="dialog-backdrop action-dialog-backdrop" @mousedown.self="emit('cancel')" @keydown.esc.stop.prevent="emit('cancel')">
    <form
      ref="dialogElement"
      class="dialog action-dialog"
      :class="`action-dialog-${state.tone}`"
      :role="state.tone === 'danger' || state.tone === 'warning' ? 'alertdialog' : 'dialog'"
      aria-modal="true"
      aria-labelledby="action-dialog-title"
      aria-describedby="action-dialog-message"
      @submit.prevent="submit"
      @keydown.tab="trapFocus"
    >
      <header>
        <span class="action-dialog-icon" aria-hidden="true"><component :is="icon" :size="18" /></span>
        <div><h2 id="action-dialog-title">{{ state.title }}</h2><p>{{ state.kind === 'prompt' ? '请确认信息后继续' : state.tone === 'danger' ? '请确认此项高风险操作' : 'Cockpit' }}</p></div>
        <button type="button" class="icon-button" aria-label="关闭" @click="emit('cancel')"><X :size="16" /></button>
      </header>
      <div class="action-dialog-body">
        <p id="action-dialog-message" class="action-dialog-message">{{ state.message }}</p>
        <p v-if="state.detail" class="action-dialog-detail">{{ state.detail }}</p>
        <label v-if="state.kind === 'prompt'" class="action-dialog-field">
          <span>{{ state.inputLabel || '输入内容' }}</span>
          <span class="action-dialog-input-wrap">
            <input
              ref="inputElement"
              v-model="input"
              :type="state.inputType === 'password' && !showPassword ? 'password' : 'text'"
              :placeholder="state.inputPlaceholder"
              :aria-invalid="touched && Boolean(validationMessage)"
              :aria-describedby="touched && validationMessage ? 'action-dialog-validation' : undefined"
              autocomplete="off"
              spellcheck="false"
              @input="touched = false"
            />
            <button v-if="state.inputType === 'password'" type="button" class="action-dialog-password-toggle" :aria-label="showPassword ? '隐藏密码' : '显示密码'" @click="showPassword = !showPassword">
              <EyeOff v-if="showPassword" :size="15" /><Eye v-else :size="15" />
            </button>
          </span>
          <small v-if="touched && validationMessage" id="action-dialog-validation" class="action-dialog-validation" role="alert">{{ validationMessage }}</small>
        </label>
      </div>
      <footer>
        <span>{{ state.tone === 'danger' ? '此操作可能无法撤销' : '' }}</span>
        <button v-if="state.kind !== 'notice'" type="button" class="secondary" @click="emit('cancel')">{{ state.cancelLabel }}</button>
        <button ref="confirmButton" :class="state.tone === 'danger' ? 'destructive-primary' : 'primary'">{{ state.confirmLabel }}</button>
      </footer>
    </form>
  </div>
</template>
