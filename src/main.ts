import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { installDialogAccessibility } from "@/lib/dialogAccessibility";
import { installTextInputProtection } from "@/lib/textInput";
import { useAppStore } from "@/stores/app";
import "./styles.css";

const isMacTauri = "__TAURI_INTERNALS__" in window && /Macintosh|Mac OS X/.test(navigator.userAgent);
if (isMacTauri) document.documentElement.classList.add("tauri-macos");

const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("Missing #app root element");
installTextInputProtection(root);
installDialogAccessibility(document);
const pinia = createPinia();
const app = createApp(App);
app.use(pinia);
const store = useAppStore(pinia);
const unexpectedErrorMessage = "界面遇到意外错误。请重试；若问题持续出现，请在设置中打开诊断日志。";
app.config.errorHandler = () => { store.error = unexpectedErrorMessage; };
window.addEventListener("error", () => { store.error = unexpectedErrorMessage; });
window.addEventListener("unhandledrejection", () => { store.error = unexpectedErrorMessage; });
app.mount(root);
