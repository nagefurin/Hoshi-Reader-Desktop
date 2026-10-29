import { subscribeForkSettings, setForkSetting } from "./forkSettingsFile";

const FORK_PREFIX = "nagefurin.fork.";
export function persisted<T extends object>(key: string, defaults: T) {
  const workKey = `${FORK_PREFIX}${key}`;

  // Seed compatible preferences from the original app without modifying its keys.
  try {
    const original = localStorage.getItem(key);
    if (original !== null) {
      const parsed: unknown = JSON.parse(original);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        localStorage.setItem(workKey, JSON.stringify(parsed));
      }
    }
  } catch {}

  let initial: T = { ...defaults };
  try {
    const stored = localStorage.getItem(workKey);
    if (stored) initial = { ...defaults, ...JSON.parse(stored) };
  } catch {}
  const config = $state<T>(initial);

  // The fork-specific JSON file is authoritative; browser storage is only used
  // to migrate existing fork preferences or seed compatible upstream preferences.
  subscribeForkSettings((document) => {
    const stored = document[key];
    if (stored && typeof stored === "object" && !Array.isArray(stored)) {
      Object.assign(config, defaults, stored);
    } else {
      setForkSetting(key, $state.snapshot(config));
    }
  });

  return {
    config,
    save() {
      const snapshot = $state.snapshot(config);
      try { localStorage.setItem(workKey, JSON.stringify(snapshot)); } catch {}
      setForkSetting(key, snapshot);
    },
  };
}
