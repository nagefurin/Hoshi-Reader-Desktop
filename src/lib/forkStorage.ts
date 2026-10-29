import { setForkSetting, subscribeForkSettings } from "./forkSettingsFile";

const FORK_PREFIX = "nagefurin.fork.";
const observedKeys = new Set<string>();

function workKey(key: string): string {
  return FORK_PREFIX + key;
}

function localStorageValue(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

export function loadForkPreference(key: string): string | null {
  observedKeys.add(key);
  try {
    const work = localStorage.getItem(workKey(key));
    if (work !== null) return work;
    const original = localStorage.getItem(key);
    if (original !== null) {
      localStorage.setItem(workKey(key), original);
      return original;
    }
  } catch {}
  return null;
}

export function saveForkPreference(key: string, value: string): void {
  observedKeys.add(key);
  try { localStorage.setItem(workKey(key), value); } catch {}
  setForkSetting(key, value);
}

subscribeForkSettings((document) => {
  for (const key of observedKeys) {
    if (Object.prototype.hasOwnProperty.call(document, key)) {
      try { localStorage.setItem(workKey(key), localStorageValue(document[key])); } catch {}
    } else {
      const fallback = loadForkPreference(key);
      if (fallback !== null) setForkSetting(key, fallback);
    }
  }
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event("nagefurin-settings-loaded"));
  }
});
