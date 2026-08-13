<script setup lang="ts">
import { computed, useSlots } from "vue";
import { PanelsTopLeft, X } from "lucide-vue-next";

const props = withDefaults(defineProps<{
  title: string;
  titleId: string;
  description?: string;
  descriptionId?: string;
  describedBy?: string;
  as?: "section" | "form";
  role?: "dialog" | "alertdialog";
  dialogId?: string;
  dialogClass?: string | string[] | Record<string, boolean>;
  backdropClass?: string | string[] | Record<string, boolean>;
  closeLabel?: string;
  closeDisabled?: boolean;
  closeOnBackdrop?: boolean;
  closeOnEscape?: boolean;
}>(), {
  description: "",
  descriptionId: undefined,
  describedBy: undefined,
  as: "section",
  role: "dialog",
  dialogId: undefined,
  dialogClass: "",
  backdropClass: "",
  closeLabel: "关闭弹窗",
  closeDisabled: false,
  closeOnBackdrop: true,
  closeOnEscape: true,
});

const emit = defineEmits<{ close: []; submit: [] }>();
const slots = useSlots();
const resolvedDescriptionId = computed(() => props.description
  ? props.descriptionId ?? `${props.titleId}-description`
  : undefined);

function close(source: "button" | "backdrop" | "escape") {
  if (source === "backdrop" && !props.closeOnBackdrop) return;
  if (source === "escape" && !props.closeOnEscape) return;
  if (props.closeDisabled) return;
  emit("close");
}
</script>

<template>
  <div
    :class="['dialog-backdrop', 'app-dialog-backdrop', backdropClass]"
    @mousedown.self="close('backdrop')"
    @keydown.esc.stop.prevent="close('escape')"
  >
    <component
      :is="as"
      :id="dialogId"
      :class="['dialog', 'app-dialog', dialogClass]"
      :role="role"
      aria-modal="true"
      :aria-labelledby="titleId"
      :aria-describedby="describedBy ?? resolvedDescriptionId"
      tabindex="-1"
      @submit.prevent="emit('submit')"
    >
      <header class="app-dialog-header">
        <div class="app-dialog-heading">
          <span class="app-dialog-heading-icon" aria-hidden="true">
            <slot name="icon"><PanelsTopLeft :size="18" :stroke-width="1.8" /></slot>
          </span>
          <div>
            <h2 :id="titleId">{{ title }}</h2>
            <p v-if="description" :id="resolvedDescriptionId">{{ description }}</p>
          </div>
        </div>
        <button type="button" class="icon-button" :aria-label="closeLabel" :disabled="closeDisabled" @click="close('button')">
          <X :size="16" />
        </button>
      </header>

      <slot />

      <footer v-if="slots.footer" class="app-dialog-footer">
        <slot name="footer" />
      </footer>
    </component>
  </div>
</template>
