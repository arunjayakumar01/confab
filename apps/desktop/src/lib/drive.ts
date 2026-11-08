import type { DriveItem, DriveWhere } from "../bridge-types";

export const FOLDER = "application/vnd.google-apps.folder";
const SHORTCUT = "application/vnd.google-apps.shortcut";

/** A literal for a Drive query: backslash and single quote escaped. */
export const lit = (s: string): string => `'${s.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;

/** The `files.list` query for a place. Google refuses orderBy on fullText searches. */
export function driveQuery(where: DriveWhere): { q: string; orderBy: string; driveId?: string } {
  const live = "trashed = false";
  switch (where.kind) {
    case "root": return { q: `'root' in parents and ${live}`, orderBy: "folder,name" };
    case "folder": return { q: `${lit(where.id)} in parents and ${live}`, orderBy: "folder,name", driveId: where.driveId };
    case "shared": return { q: `sharedWithMe and ${live}`, orderBy: "folder,name" };
    case "starred": return { q: `starred and ${live}`, orderBy: "folder,name" };
    case "recent": return { q: `${live} and mimeType != ${lit(FOLDER)}`, orderBy: "modifiedTime desc" };
    case "trash": return { q: "trashed = true", orderBy: "modifiedTime desc" };
    case "search": return { q: `fullText contains ${lit(where.query)} and ${live}`, orderBy: "" };
    case "drives": return { q: "", orderBy: "" };
  }
}

export const isFolder = (f: Pick<DriveItem, "mimeType" | "shortcutDetails">): boolean =>
  f.mimeType === FOLDER || (f.mimeType === SHORTCUT && f.shortcutDetails?.targetMimeType === FOLDER);
export const isShortcut = (f: DriveItem): boolean => f.mimeType === SHORTCUT;
/** Google Docs, Sheets, Slides and friends: no bytes of their own, they export as PDF. */
export const isGoogleFile = (mime: string): boolean => mime.startsWith("application/vnd.google-apps.") && mime !== FOLDER && mime !== SHORTCUT;

export function humanSize(bytes?: string): string {
  const n = Number(bytes);
  if (!bytes || !Number.isFinite(n)) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** The id a click should open: a shortcut's target, else the item itself. */
export const targetId = (f: DriveItem): string => f.shortcutDetails?.targetId ?? f.id;
