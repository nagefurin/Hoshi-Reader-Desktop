import { subscribeForkSettings, setForkSetting } from "./forkSettingsFile";

const FORK_PREFIX = "nagefurin.fork.";
// TTS is a fork-only feature: never seed or overwrite it from the upstream app.
const FORK_ONLY_SETTINGS = new Set(["tts.config"]);

export function persisted<T extends object>(key: string, defaults: T) {
  const workKey = `${FORK_PREFIX}${key}`;

  // Refresh compatible settings from the original app at startup, without
  // ever writing to the original app's unprefixed keys.
  if (!FORK_ONLY_SETTINGS.has(key)) {
    try {
      const original = localStorage.getItem(key);
      if (original !== null) {
        const parsed: unknown = JSON.parse(original);
        if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
          localStorage.setItem(workKey, JSON.stringify(parsed));
        }
      }
    } catch {}
  } else {
    // One-time migration of this fork's older, unprefixed TTS settings.
    // After migration, TTS settings belong solely to the fork-specific key.
    try {
      if (localStorage.getItem(workKey) === null) {
        const legacy = localStorage.getItem(key);
        if (legacy !== null) {
          const parsed: unknown = JSON.parse(legacy);
          if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
            localStorage.setItem(workKey, JSON.stringify(parsed));
          }
        }
      }
    } catch {}
  }

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
