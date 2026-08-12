<script setup lang="ts" generic="T extends SelectValue = SelectValue">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch, type CSSProperties } from "vue";
import { Listbox, ListboxButton, ListboxOption, ListboxOptions } from "@headlessui/vue";
import { Check, ChevronDown } from "lucide-vue-next";

export type SelectValue = string | number | null | undefined;
export interface SelectOption {
  value: SelectValue;
  label: string;
  disabled?: boolean;
}

const props = withDefaults(defineProps<{
  modelValue: T;
  options: SelectOption[];
  label: string;
  placeholder?: string;
  emptyLabel?: string;
  disabled?: boolean;
  variant?: "default" | "compact" | "cell" | "context" | "toolbar";
  menuMinWidth?: number;
}>(), {
  placeholder: "请选择",
  emptyLabel: "暂无可用项",
  disabled: false,
  variant: "default",
  menuMinWidth: 180,
});

const emit = defineEmits<{
  "update:modelValue": [value: T];
  change: [value: T];
}>();

const root = ref<HTMLElement | null>(null);
const menuId = `app-select-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
const menuStyle = ref<CSSProperties>({ visibility: "hidden" });
const selectedOption = computed(() => props.options.find((option) => Object.is(option.value, props.modelValue)));
const selectedLabel = computed(() => selectedOption.value?.label ?? props.placeholder);

function selectValue(value: unknown) {
  const option = props.options.find((item) => Object.is(item.value, value));
  if (!option || option.disabled) return;
  const nextValue = option.value as T;
  emit("update:modelValue", nextValue);
  emit("change", nextValue);
}

function updateMenuPosition() {
  const button = root.value?.querySelector<HTMLButtonElement>(".app-select-button");
  const menu = document.getElementById(menuId);
  if (!button || !menu) return;
  const rect = button.getBoundingClientRect();
  const viewportMargin = 6;
  const width = Math.min(
    Math.max(rect.width, props.menuMinWidth),
    Math.max(0, window.innerWidth - viewportMargin * 2),
  );
  const left = Math.min(
    Math.max(viewportMargin, rect.left),
    Math.max(viewportMargin, window.innerWidth - width - viewportMargin),
  );
  const menuHeight = Math.min(menu.scrollHeight, 240);
  const spaceBelow = window.innerHeight - rect.bottom;
  const openAbove = spaceBelow < Math.min(menuHeight, 160) && rect.top > spaceBelow;
  menuStyle.value = {
    visibility: "visible",
    left: `${Math.round(left)}px`,
    top: `${Math.round(openAbove ? Math.max(viewportMargin, rect.top - menuHeight - 5) : rect.bottom + 5)}px`,
    width: `${Math.round(width)}px`,
  };
}

function scheduleMenuPosition() {
  menuStyle.value = { visibility: "hidden" };
  void nextTick(() => requestAnimationFrame(updateMenuPosition));
}

function handleTriggerKeydown(event: KeyboardEvent) {
  if (["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) scheduleMenuPosition();
}

function refreshOpenMenuPosition() {
  if (document.getElementById(menuId)) updateMenuPosition();
}

watch(() => props.options.length, () => {
  if (document.getElementById(menuId)) scheduleMenuPosition();
});

onMounted(() => {
  window.addEventListener("resize", refreshOpenMenuPosition);
  window.addEventListener("scroll", refreshOpenMenuPosition, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", refreshOpenMenuPosition);
  window.removeEventListener("scroll", refreshOpenMenuPosition, true);
});
</script>

<template>
  <div ref="root" class="app-select" :class="`app-select-${variant}`">
    <Listbox
      as="div"
      class="app-select-control"
      :model-value="modelValue"
      :disabled="disabled"
      :nullable="true"
      @update:model-value="selectValue"
    >
      <ListboxButton
        class="app-select-button"
        :aria-label="label"
        :data-value="modelValue ?? ''"
        @click.stop="scheduleMenuPosition"
        @keydown="handleTriggerKeydown"
      >
        <span :class="{ placeholder: !selectedOption }">{{ selectedLabel }}</span>
        <ChevronDown :size="13" aria-hidden="true" />
      </ListboxButton>
      <Teleport to="body">
        <Transition
          enter-active-class="app-select-options-enter-active"
          enter-from-class="app-select-options-enter-from"
          leave-active-class="app-select-options-leave-active"
          leave-to-class="app-select-options-leave-to"
        >
          <ListboxOptions :id="menuId" class="app-select-options" :aria-label="label" :style="menuStyle">
            <li v-if="!options.length" class="app-select-empty">{{ emptyLabel }}</li>
            <ListboxOption
              v-for="option in options"
              :key="`${typeof option.value}:${String(option.value)}`"
              v-slot="{ active, selected, disabled: optionDisabled }"
              as="template"
              :value="option.value"
              :disabled="option.disabled"
            >
              <li
                class="app-select-option"
                :class="{ active, selected, disabled: optionDisabled }"
              >
                <span>{{ option.label }}</span>
                <Check v-if="selected" :size="13" aria-hidden="true" />
              </li>
            </ListboxOption>
          </ListboxOptions>
        </Transition>
      </Teleport>
    </Listbox>
  </div>
</template>

<style scoped>
.app-select, .app-select-control { width: 100%; min-width: 0; }
.app-select-button { position: relative; width: 100%; min-width: 0; min-height: 34px; justify-content: space-between; gap: 7px; padding: 0 8px 0 10px; overflow: hidden; border: 1px solid var(--border-strong); border-radius: var(--radius-sm); background: var(--surface-1); color: var(--text); font-size: 11px; font-weight: 500; }
.app-select-button:hover:not(:disabled) { border-color: var(--accent); background: var(--surface-hover); }
.app-select-button:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
.app-select-button:disabled { color: var(--muted); cursor: default; }
.app-select-button > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.app-select-button > span.placeholder { color: var(--muted); font-weight: 500; }
.app-select-button > svg { flex: 0 0 auto; color: var(--muted); transition: transform 120ms ease; }
.app-select-button[aria-expanded="true"] > svg { transform: rotate(180deg); }
.app-select-compact .app-select-button { min-height: 30px; height: 30px; padding-inline: 7px; }
.app-select-cell .app-select-button { min-height: 23px; height: 23px; padding: 2px 6px; border-color: transparent; border-radius: 0; background: transparent; font: inherit; }
.app-select-cell .app-select-button:focus-visible { border-color: var(--accent); outline: 0; background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--accent); }
.app-select-context .app-select-button { min-height: 26px; height: 26px; padding: 0; border: 0; border-radius: 0; background: transparent; font-size: 10.5px; font-weight: 600; }
.app-select-context .app-select-button:hover { background: transparent; }
.app-select-context .app-select-button:focus-visible { outline: 0; }
.app-select-toolbar { width: 84px; height: 22px; flex: 0 0 auto; align-self: center; }
.app-select-toolbar .app-select-control, .app-select-toolbar .app-select-button { height: 22px; min-height: 22px; }
.app-select-toolbar .app-select-button { padding: 0 18px 0 7px; border: 0; border-radius: 0; background: var(--surface-1); font-size: 10.5px; line-height: 1; }
.app-select-options { position: fixed; z-index: 260; max-height: 240px; margin: 0; padding: 4px; overflow-y: auto; border: 1px solid var(--border-strong); border-radius: var(--radius-md); outline: 0; background: var(--surface-1); box-shadow: var(--shadow-md); list-style: none; scrollbar-color: var(--border-strong) transparent; scrollbar-width: thin; }
.app-select-option, .app-select-empty { min-height: 28px; display: flex; align-items: center; gap: 8px; padding: 5px 7px; border-radius: var(--radius-sm); font-size: 10.5px; }
.app-select-option { justify-content: space-between; cursor: pointer; }
.app-select-option > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.app-select-option > svg { flex: 0 0 auto; color: var(--accent); }
.app-select-option.active { background: var(--surface-hover); }
.app-select-option.selected { color: var(--accent); font-weight: 650; }
.app-select-option.disabled { color: var(--muted); cursor: default; opacity: .55; }
.app-select-empty { color: var(--muted); }
.app-select-options-enter-active, .app-select-options-leave-active { transition: opacity 100ms ease, transform 100ms ease; transform-origin: top; }
.app-select-options-enter-from, .app-select-options-leave-to { opacity: 0; transform: translateY(-3px) scale(.98); }
</style>
