import { describe, expect, it } from "vitest";
import { driveQuery, humanSize, isFolder, lit } from "../lib/drive";

describe("drive queries", () => {
  it("escapes literals and builds a query per place", () => {
    expect(lit("it's a\\b")).toBe("'it\\'s a\\\\b'");
    expect(driveQuery({ kind: "root" })).toEqual({ q: "'root' in parents and trashed = false", orderBy: "folder,name" });
    expect(driveQuery({ kind: "folder", id: "f1", driveId: "d1" })).toEqual({ q: "'f1' in parents and trashed = false", orderBy: "folder,name", driveId: "d1" });
    expect(driveQuery({ kind: "trash" }).q).toBe("trashed = true");
    // Google refuses orderBy with fullText.
    expect(driveQuery({ kind: "search", query: "Q4 'plan'" })).toEqual({ q: "fullText contains 'Q4 \\'plan\\'' and trashed = false", orderBy: "" });
  });
  it("reads types and sizes", () => {
    expect(isFolder({ mimeType: "application/vnd.google-apps.folder" })).toBe(true);
    expect(isFolder({ mimeType: "application/vnd.google-apps.shortcut", shortcutDetails: { targetId: "x", targetMimeType: "application/vnd.google-apps.folder" } })).toBe(true);
    expect([humanSize("512"), humanSize("2048"), humanSize("5242880"), humanSize(undefined)]).toEqual(["512 B", "2.0 KB", "5.0 MB", ""]);
  });
});
