import { invoke } from "@tauri-apps/api/core";

export type ForkSettingsDocument = Record<string, unknown>;
type Listener = (document: ForkSettingsDocument) => void;

let document: ForkSettingsDocument = {};
let ready = false;
const listeners = new Set<Listener>();
const pending = new Map<string, unknown>();
let writeQueue: Promise<unknown> = Promise.resolve();

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function persistDocument() {
  const snapshot = JSON.parse(JSON.stringify(document)) as ForkSettingsDocument;
  writeQueue = writeQueue
    .catch(() => undefined)
    .then(() => invoke("save_fork_settings", { settings: snapshot }))
    .catch((error) => console.error("Could not save nagefurin-setting.json:", error));
}

function notify() {
  for (const listener of listeners) listener(document);
}

const loading = invoke<unknown>("load_fork_settings")
  .then((value) => {
    document = isObject(value) ? value : {};
    ready = true;
    let changed = false;
    for (const [key, candidate] of pending) {
      // Existing file values win over startup-time default saves.
      if (!Object.prototype.hasOwnProperty.call(document, key)) {
        document[key] = candidate;
        changed = true;
      }
    }
    pending.clear();
    notify();
    if (changed) persistDocument();
  })
  .catch((error) => {
    console.error("Could not load nagefurin-setting.json; using migration values:", error);
    document = {};
    ready = true;
    for (const [key, candidate] of pending) document[key] = candidate;
    pending.clear();
    notify();
    persistDocument();
  });

export function subscribeForkSettings(listener: Listener): () => void {
  listeners.add(listener);
  if (ready) listener(document);
  else void loading.then(() => { if (listeners.has(listener)) listener(document); });
  return () => listeners.delete(listener);
}

export function setForkSetting(key: string, value: unknown): void {
  if (!ready) {
    pending.set(key, value);
    return;
  }
  document[key] = value;
  persistDocument();
}
