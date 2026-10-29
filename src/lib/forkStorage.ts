const FORK_PREFIX = "nagefurin.fork.";

// Import the original value at startup, then read and write only the fork's key.
export function loadForkPreference(key: string): string | null {
  const workKey = `${FORK_PREFIX}${key}`;
  try {
    const original = localStorage.getItem(key);
    if (original !== null) localStorage.setItem(workKey, original);
    return localStorage.getItem(workKey);
  } catch {
    return null;
  }
}

export function saveForkPreference(key: string, value: string): void {
  localStorage.setItem(`${FORK_PREFIX}${key}`, value);
}
