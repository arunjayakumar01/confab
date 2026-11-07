import type { Bridge } from "../bridge-types";

export type ShellKind = "desktop" | "browser" | "mock";

export function detectShell(): ShellKind {
  if (import.meta.env.VITE_BRIDGE === "mock") return "mock";
  return "__TAURI_INTERNALS__" in window ? "desktop" : "browser";
}

let instance: Bridge | null = null;
export async function getBridge(): Promise<Bridge> {
  if (instance) return instance;
  const kind = detectShell();
  if (kind === "desktop") instance = (await import("./desktop")).createDesktopBridge();
  else if (kind === "mock") instance = (await import("./mock")).createMockBridge();
  else instance = (await import("./browser")).createBrowserBridge();
  return instance;
}
