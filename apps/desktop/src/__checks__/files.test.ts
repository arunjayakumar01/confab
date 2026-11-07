import { describe, expect, it } from "vitest";
import { driveFromText, fileKind, isInlineImage, needsResolve, previewKind } from "../lib/files";

describe("fileKind", () => {
  it("reads Google types, then MIME families, then the extension", () => {
    expect(fileKind("application/vnd.google-apps.document").label).toBe("DOC");
    expect(fileKind("application/pdf", "x.bin").label).toBe("PDF");
    expect(fileKind(undefined, "notes.PDF").label).toBe("PDF");
    expect(fileKind("application/octet-stream", "build.tgz").label).toBe("ZIP");
    expect(fileKind("application/octet-stream", "weird.blend").label).toBe("BLEND");
    expect(fileKind(undefined, undefined).label).toBe("FILE");
  });
  it("tells inline images from cards and bare Drive stubs from full resources", () => {
    expect(isInlineImage({ contentType: "image/png", thumbnailUri: "x" })).toBe(true);
    expect(isInlineImage({ contentType: "image/png" })).toBe(false);
    expect(needsResolve({ name: "spaces/a/messages/b/attachments/c", source: "DRIVE_FILE" })).toBe(true);
    expect(needsResolve({ name: "n", contentName: "a.pdf" })).toBe(false);
  });
  it("picks a viewer by MIME first and extension second, none for what nothing renders", () => {
    expect(previewKind("application/pdf")).toBe("pdf");
    expect(previewKind(undefined, "notes.MD")).toBe("text");
    expect(previewKind("application/octet-stream", "relay.log")).toBe("text");
    expect(previewKind("video/mp4")).toBe("video");
    expect(previewKind("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "a.docx")).toBe("docx");
    expect(previewKind("application/octet-stream", "budget.xlsx")).toBe("sheet");
    expect(previewKind("text/csv", "rows.csv")).toBe("sheet");
    expect(previewKind("application/zip", "a.zip")).toBe("zip");
    expect(previewKind("application/octet-stream", "deck.pptx")).toBe("pptx");
    expect(previewKind("application/x-tar", "a.tgz")).toBeNull();
  });
  it("fills a Drive stub from the share message's link, matching the file id when there are several", () => {
    const text = "Ragavendran invited you to contribute to\n[Inmac\nDocumentations](https://drive.google.com/drive/folders/1HLt?ts=1&usp=x) and [Budget](https://docs.google.com/spreadsheets/d/9ZZ/edit)";
    const a = { name: "spaces/a/messages/b/attachments/c", source: "DRIVE_FILE" as const, driveDataRef: { driveFileId: "9ZZ" } };
    const full = driveFromText(a, text)!;
    expect(full.contentName).toBe("Budget");
    expect(full.contentType).toBe("application/vnd.google-apps.spreadsheet");
    expect(full.downloadUri).toContain("docs.google.com/spreadsheets");
    expect(driveFromText({ ...a, driveDataRef: undefined }, text)!.contentName).toBe("Inmac Documentations");
    expect(driveFromText(a, "no links here")).toBeNull();
    expect(driveFromText({ name: "x", source: "UPLOADED_CONTENT" }, text)).toBeNull();
  });
});
