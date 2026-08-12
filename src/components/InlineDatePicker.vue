<script setup lang="ts">
import { computed, onMounted, onUpdated, ref } from "vue";
import { VueDatePicker } from "@vuepic/vue-datepicker";
import { zhCN } from "date-fns/locale";
import "@vuepic/vue-datepicker/dist/main.css";

const props = withDefaults(defineProps<{
  modelValue: string;
  kind: "text" | "date" | "datetime-local";
  columnName: string;
  inputLabel: string;
  placement?: "top-start" | "top-end" | "bottom-start" | "bottom-end";
  disabled?: boolean;
  placeholder?: string;
  isNull?: boolean;
  useDefault?: boolean;
}>(), {
  placement: "bottom-start",
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  focus: [];
  tab: [event: KeyboardEvent];
  enter: [event: KeyboardEvent];
  escape: [event: KeyboardEvent];
}>();

const root = ref<HTMLElement | null>(null);
const picker = ref<{ closeMenu: () => void } | null>(null);
const menuOpen = ref(false);

const includesTime = computed(() => props.kind === "datetime-local");
const displayFormat = computed(() => includesTime.value ? "yyyy-MM-dd HH:mm:ss" : "yyyy-MM-dd");
const pickerValue = computed(() => parseValue(props.modelValue));
const inputClass = computed(() => [
  "inline-cell-input",
  props.isNull ? "null" : "",
  props.useDefault ? "default" : "",
]);

const floating = {
  strategy: "fixed" as const,
  placement: props.placement,
  offset: 6,
  flip: { fallbackPlacements: ["top-start" as const], rootBoundary: "viewport" as const, padding: 8 },
  shift: { rootBoundary: "viewport" as const, padding: 8 },
};
const pickerConfig = { allowPreventDefault: true };

function parseValue(value: string) {
  const matched = value.match(/^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/);
  if (!matched) return null;
  const date = new Date(
    Number(matched[1]),
    Number(matched[2]) - 1,
    Number(matched[3]),
    Number(matched[4] ?? 0),
    Number(matched[5] ?? 0),
    Number(matched[6] ?? 0),
  );
  return Number.isNaN(date.getTime()) ? null : date;
}

function pad(value: number) {
  return String(value).padStart(2, "0");
}

function formatValue(value: unknown) {
  if (!(value instanceof Date) || Number.isNaN(value.getTime())) return "";
  const date = `${value.getFullYear()}-${pad(value.getMonth() + 1)}-${pad(value.getDate())}`;
  if (!includesTime.value) return date;
  return `${date} ${pad(value.getHours())}:${pad(value.getMinutes())}:${pad(value.getSeconds())}`;
}

function updateValue(value: unknown) {
  emit("update:modelValue", formatValue(value));
}

function tagInput() {
  const input = root.value?.querySelector<HTMLInputElement>("input");
  if (!input) return;
  input.dataset.column = props.columnName;
  input.setAttribute("aria-label", props.inputLabel);
}

function handleKeydown(event: KeyboardEvent) {
  if (event.key === "Tab" && !event.shiftKey) {
    event.preventDefault();
    event.stopPropagation();
    picker.value?.closeMenu();
    emit("tab", event);
  } else if (event.key === "Enter" && !menuOpen.value) {
    event.preventDefault();
    event.stopPropagation();
    emit("enter", event);
  } else if (event.key === "Escape" && !menuOpen.value) {
    event.preventDefault();
    event.stopPropagation();
    emit("escape", event);
  }
}

onMounted(tagInput);
onUpdated(tagInput);
</script>

<template>
  <div
    ref="root"
    class="inline-date-picker"
    :data-column="columnName"
    :data-placement="placement"
    @click.stop
    @keydown="handleKeydown"
  >
    <VueDatePicker
      ref="picker"
      :model-value="pickerValue"
      :formats="{ input: displayFormat, preview: displayFormat }"
      :time-config="{ enableTimePicker: includesTime, enableSeconds: includesTime, is24: true }"
      :text-input="{ format: displayFormat, enterSubmit: true, tabSubmit: true, openMenu: 'open', selectOnFocus: true }"
      :input-attrs="{ autocomplete: 'off', clearable: false, alwaysClearable: false, hideInputIcon: false, inputmode: 'none' }"
      :ui="{ input: inputClass }"
      :floating="floating"
      :config="pickerConfig"
      :teleport="true"
      :auto-apply="true"
      :disabled="disabled"
      :placeholder="placeholder"
      :locale="zhCN"
      @update:model-value="updateValue"
      @open="menuOpen = true"
      @closed="menuOpen = false"
      @focus="emit('focus')"
    />
  </div>
</template>
