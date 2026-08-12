<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";

const props = defineProps<{ x: number; y: number }>();
const emit = defineEmits<{ close: [] }>();
const menu = ref<HTMLElement | null>(null);
const position = ref({ left: props.x, top: props.y });

function closeOnEscape(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

function close() {
  emit("close");
}

function menuItems() {
  return menu.value ? Array.from(menu.value.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')) : [];
}

function handleMenuKeydown(event: KeyboardEvent) {
  const items = menuItems();
  if (!items.length) return;
  if (event.key === "Tab") {
    emit("close");
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const current = items.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home"
    ? 0
    : event.key === "End"
      ? items.length - 1
      : event.key === "ArrowDown"
        ? (current + 1 + items.length) % items.length
        : (current - 1 + items.length) % items.length;
  items[next]?.focus();
}

onMounted(async () => {
  window.addEventListener("keydown", closeOnEscape);
  window.addEventListener("resize", close);
  window.addEventListener("blur", close);
  window.addEventListener("scroll", close, true);

  await nextTick();
  if (!menu.value) return;
  const bounds = menu.value.getBoundingClientRect();
  position.value = {
    left: Math.max(8, Math.min(props.x, window.innerWidth - bounds.width - 8)),
    top: Math.max(8, Math.min(props.y, window.innerHeight - bounds.height - 8)),
  };
  menu.value.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", closeOnEscape);
  window.removeEventListener("resize", close);
  window.removeEventListener("blur", close);
  window.removeEventListener("scroll", close, true);
});
</script>

<template>
  <Teleport to="body">
    <div class="context-menu-layer" @pointerdown.self="close" @contextmenu.prevent="close">
      <div
        ref="menu"
        class="context-menu"
        role="menu"
        :style="{ left: `${position.left}px`, top: `${position.top}px` }"
        @pointerdown.stop
        @keydown="handleMenuKeydown"
      >
        <slot />
      </div>
    </div>
  </Teleport>
</template>
