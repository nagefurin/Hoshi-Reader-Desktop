import { invoke } from "@tauri-apps/api/core";
import { clearCustomShelf, renameCustomShelf } from "./bookSort.svelte";
import { series } from "./series";
import { seriesStore } from "./seriesStore.svelte";
import { shellConfig } from "./shellConfig.svelte";
import type { BookMetadata, BookShelf, ShelfSource } from "./types";
import { loadForkPreference, saveForkPreference } from "./forkStorage";

const LEGACY_ORDERS_KEY = "bookshelfShelfOrders";
const FORK_ORDERS_KEY = "bookshelfShelfOrders";

function storedOrders() {
  try {
    return JSON.parse(loadForkPreference(FORK_ORDERS_KEY) ?? "{}") as Record<string, string[]>;
  } catch {
    return {};
  }
}

export const shelves = $state<{ list: BookShelf[]; orders: Record<string, string[]> }>({
  list: [],
  orders: storedOrders(),
});

let shelfOrdersMigrated = false;

export async function loadShelves() {
  [shelves.list, shelves.orders] = await Promise.all([
    invoke<BookShelf[]>("load_shelves"),
    invoke<Record<string, string[]>>("shelf_orders"),
  ]);
}

export function saveShelfOrders() {
  saveForkPreference(FORK_ORDERS_KEY, JSON.stringify($state.snapshot(shelves.orders)));
}

export async function migrateShelfOrders(books: BookMetadata[]) {
  const sources: Record<string, string[]>[] = [];
  const forkOrders = storedOrders();
  if (Object.keys(forkOrders).length) sources.push(forkOrders);

  try {
    const legacy = localStorage.getItem(LEGACY_ORDERS_KEY);
    if (legacy) {
      const parsed = JSON.parse(legacy) as Record<string, string[]>;
      if (Object.keys(parsed).length) sources.push(parsed);
    }
  } catch {
    // Ignore malformed legacy preferences and keep the current database order.
  }

  const folders = new Map(books.map((book) => [book.id, book.folder]));
  for (const source of sources) {
    for (const [name, ids] of Object.entries(source)) {
      if (shelves.orders[name] || !shelves.list.some((shelf) => shelf.name === name)) continue;
      // TTS-era preferences stored book IDs; newer mirrored preferences store folder names.
      const order = ids.flatMap((id) => (folders.has(id) ? folders.get(id) ?? [] : [id]));
      if (order.length) {
        await invoke("set_shelf_order", { name, order });
        shelves.orders[name] = order;
      }
    }
  }

  try {
    localStorage.removeItem(LEGACY_ORDERS_KEY);
  } catch {
    // The fork settings file remains the persistent copy if localStorage is unavailable.
  }

  await loadShelves();
  saveShelfOrders();
  shelfOrdersMigrated = true;
}

export async function saveShelfOrder(name: string) {
  await invoke("set_shelf_order", { name, order: $state.snapshot(shelves.orders[name]) ?? null });
  saveShelfOrders();
}

export async function resetShelfOrder(name: string) {
  clearCustomShelf(name);
  await invoke("set_shelf_order", { name, order: null });
  await loadShelves();
  saveShelfOrders();
}

export async function createShelf(name: string) {
  if (shelves.list.some((shelf) => shelf.name === name)) return;
  await invoke("create_shelf", { name });
  await loadShelves();
}

export async function deleteShelf(name: string) {
  clearCustomShelf(name);
  await invoke("delete_shelf", { name });
  await loadShelves();
  saveShelfOrders();
}

export async function renameShelf(name: string, newName: string) {
  await invoke("rename_shelf", { name, newName });
  renameCustomShelf(name, newName);
  await loadShelves();
  saveShelfOrders();
}

export function moveShelves() {
  invoke("move_shelves", { names: shelves.list.map((shelf) => shelf.name) });
}

export function setShelfOrder(name: string, folders: string[]) {
  shelves.orders[name] = folders;
}

export async function moveBook(id: string, name: string | null) {
  await invoke("move_book", { id, name });
  await loadShelves();
}

function isReading(book: BookMetadata) {
  return book.progress > 0 && !isFinished(book);
}

export function isCompleted(book: BookMetadata) {
  return book.completed?.value != null;
}

export function isFinished(book: BookMetadata) {
  return isCompleted(book) || book.progress >= 0.999;
}

export function completionDate(book: BookMetadata) {
  const value = book.completed?.value;
  return value ? new Date(value) : null;
}

function authorKey(name: string) {
  return name.replace(/\s/g, "");
}

export function shelfBooks(books: BookMetadata[], source: ShelfSource) {
  switch (source.kind) {
    case "home":
      return books.filter(isReading);
    case "shelf": {
      const ids = new Set(shelves.list.find((shelf) => shelf.name === source.name)?.bookIds ?? []);
      return books.filter((book) => ids.has(book.id));
    }
    case "unshelved": {
      const ids = new Set(shelves.list.flatMap((shelf) => shelf.bookIds));
      return books.filter((book) => !ids.has(book.id) && !(shellConfig.showCompleted && isFinished(book)));
    }
    case "completed":
      return books.filter(isFinished);
    case "series":
      return series(books, seriesStore.names).find((group) => group.key === source.key)?.books ?? [];
    case "author": {
      const key = authorKey(source.name);
      return books.filter((book) => book.author && authorKey(book.author) === key);
    }
    default:
      return books;
  }
}

export function sourceTitle(source: ShelfSource) {
  if ("name" in source) return source.name;
  if (source.kind === "completed") return "Completed";
  return source.kind === "unshelved" ? "Unshelved" : "All Books";
}

export function sameSource(a: ShelfSource, b: ShelfSource) {
  if (a.kind !== b.kind) return false;
  if (a.kind === "author" && b.kind === "author") return authorKey(a.name) === authorKey(b.name);
  if (a.kind === "series" && b.kind === "series") return a.key === b.key;
  return !("name" in a) || a.name === (b as { name: string }).name;
}

export function authors(books: BookMetadata[]) {
  const counts = new Map<string, number>();
  for (const book of books) {
    if (book.author) counts.set(book.author, (counts.get(book.author) ?? 0) + 1);
  }
  const groups = new Map<string, { name: string; count: number }>();
  for (const [name, count] of counts) {
    const key = authorKey(name);
    const group = groups.get(key);
    if (group) {
      group.count += count;
      if (count > counts.get(group.name)!) group.name = name;
    } else {
      groups.set(key, { name, count });
    }
  }
  return [...groups.values()]
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
}

async function refreshShelfOrdersFromForkFile() {
  if (!shelfOrdersMigrated) return;
  const stored = storedOrders();
  const names = new Set([...Object.keys(shelves.orders), ...Object.keys(stored)]);
  for (const name of names) {
    await invoke("set_shelf_order", { name, order: stored[name] ?? null });
  }
  await loadShelves();
}

if (typeof window !== "undefined") {
  window.addEventListener("nagefurin-settings-loaded", () => {
    void refreshShelfOrdersFromForkFile();
  });
}
