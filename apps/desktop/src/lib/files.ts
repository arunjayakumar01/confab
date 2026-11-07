import type { Attachment } from "../bridge-types";

export interface FileKind { label: string; color: string; name: string }

const GOOGLE: Record<string, [string, string, string]> = {
  "application/vnd.google-apps.document": ["DOC", "#4285f4", "Google Doc"],
  "application/vnd.google-apps.spreadsheet": ["SHEET", "#0f9d58", "Google Sheet"],
  "application/vnd.google-apps.presentation": ["SLIDES", "#f4b400", "Google Slides"],
  "application/vnd.google-apps.form": ["FORM", "#7248b9", "Google Form"],
  "application/vnd.google-apps.drawing": ["DRAW", "#d93025", "Google Drawing"],
  "application/vnd.google-apps.folder": ["FOLDER", "#5f6368", "Drive folder"],
};

/** A short badge and colour for a file card, from the MIME type first and the extension second. */
export function fileKind(contentType?: string, name?: string): FileKind {
  const ct = (contentType ?? "").toLowerCase();
  const g = GOOGLE[ct];
  if (g) return { label: g[0], color: g[1], name: g[2] };
  const ext = (name ?? "").includes(".") ? name!.slice(name!.lastIndexOf(".") + 1).toLowerCase() : "";
  if (ct === "application/pdf" || ext === "pdf") return { label: "PDF", color: "#d93025", name: "PDF" };
  if (ct.startsWith("image/") || ["png", "jpg", "jpeg", "gif", "webp", "svg", "heic"].includes(ext)) return { label: "IMG", color: "#7b6cff", name: "Image" };
  if (ct.startsWith("video/") || ["mp4", "mov", "webm", "mkv"].includes(ext)) return { label: "VIDEO", color: "#e37400", name: "Video" };
  if (ct.startsWith("audio/") || ["mp3", "wav", "m4a", "aac", "ogg"].includes(ext)) return { label: "AUDIO", color: "#2ec4b6", name: "Audio" };
  if (/zip|tar|gzip|7z|rar|compressed/.test(ct) || ["zip", "gz", "tgz", "7z", "rar"].includes(ext)) return { label: "ZIP", color: "#5f6368", name: "Archive" };
  if (/wordprocessing|msword/.test(ct) || ["doc", "docx"].includes(ext)) return { label: "DOC", color: "#2b579a", name: "Word document" };
  if (/spreadsheet|excel/.test(ct) || ["xls", "xlsx", "csv"].includes(ext)) return { label: "SHEET", color: "#217346", name: "Spreadsheet" };
  if (/presentation|powerpoint/.test(ct) || ["ppt", "pptx", "key"].includes(ext)) return { label: "SLIDES", color: "#d24726", name: "Presentation" };
  if (ct.startsWith("text/") || ["txt", "md", "json", "log"].includes(ext)) return { label: "TXT", color: "#6b6b7a", name: "Text" };
  if (ext) return { label: ext.slice(0, 5).toUpperCase(), color: "#6b6b7a", name: `${ext.toUpperCase()} file` };
  return { label: "FILE", color: "#6b6b7a", name: "File" };
}

/** Inline image candidate: an image type with something to load. */
export const isInlineImage = (a: Attachment): boolean =>
  (a.contentType ?? "").startsWith("image/") && !!(a.thumbnailUri ?? a.downloadUri);

/** Real Google lists Drive attachments as `{name, source}` only; the card needs a fetch to fill in. */
export const needsResolve = (a: Attachment): boolean => !a.contentName && !a.contentType && !!a.name;

const DRIVE_KINDS: [RegExp, string][] = [
  [/\/document\/d\//, "application/vnd.google-apps.document"],
  [/\/spreadsheets\/d\//, "application/vnd.google-apps.spreadsheet"],
  [/\/presentation\/d\//, "application/vnd.google-apps.presentation"],
  [/\/forms\/d\//, "application/vnd.google-apps.form"],
  [/\/drawings\/d\//, "application/vnd.google-apps.drawing"],
  [/\/drive\/folders\//, "application/vnd.google-apps.folder"],
];
const LINK = /\[([^\]]+)\]\((https?:\/\/(?:drive|docs)\.google\.com\/[^)\s]+)\)/g;

/**
 * Google refuses `attachments.get` for user auth, but a Drive share message's `formattedText` carries the
 * file as a markdown link whose URL holds the file id. Returns the filled-in attachment, or null when the
 * text has no usable link (the caller may still try the API, which the twin answers).
 */
export function driveFromText(a: Attachment, formattedText?: string): Attachment | null {
  if (a.source !== "DRIVE_FILE" || !formattedText) return null;
  const id = a.driveDataRef?.driveFileId;
  const links = [...formattedText.matchAll(LINK)].map((m) => ({ label: m[1]!, url: m[2]! }));
  const hit = (id && links.find((l) => l.url.includes(id))) || links[0];
  if (!hit) return null;
  const mime = DRIVE_KINDS.find(([re]) => re.test(hit.url))?.[1];
  return { ...a, contentName: hit.label.replace(/\s+/g, " ").trim(), contentType: mime, downloadUri: hit.url, driveDataRef: a.driveDataRef ?? (id ? { driveFileId: id } : undefined) };
}

export type PreviewKind = "image" | "pdf" | "video" | "audio" | "text" | "docx" | "sheet" | "zip" | "pptx";
const TEXT_EXT = ["txt", "md", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini", "env", "sh", "js", "ts", "tsx", "jsx", "py", "rs", "go", "rb", "java", "kt", "swift", "c", "h", "cpp", "css", "html", "sql"];
/** What the in-app viewer can show for a file; null means save or open it elsewhere. */
export function previewKind(contentType?: string, name?: string): PreviewKind | null {
  const ct = (contentType ?? "").toLowerCase();
  const ext = (name ?? "").includes(".") ? name!.slice(name!.lastIndexOf(".") + 1).toLowerCase() : "";
  if (ct.startsWith("image/") || ["png", "jpg", "jpeg", "gif", "webp", "svg"].includes(ext)) return "image";
  if (ct === "application/pdf" || ext === "pdf") return "pdf";
  if (ct.startsWith("video/") || ["mp4", "mov", "webm"].includes(ext)) return "video";
  if (ct.startsWith("audio/") || ["mp3", "wav", "m4a", "aac", "ogg"].includes(ext)) return "audio";
  if (ext === "csv" || ext === "tsv" || ct === "text/csv") return "sheet";
  if (ct.startsWith("text/") || /^application\/(json|xml|x-yaml|yaml|javascript|x-sh)$|\+(json|xml)$/.test(ct) || TEXT_EXT.includes(ext)) return "text";
  if (/wordprocessingml\.document|msword/.test(ct) || ["docx", "doc"].includes(ext)) return "docx";
  if (/spreadsheetml|ms-excel/.test(ct) || ["xlsx", "xls", "xlsm", "ods"].includes(ext)) return "sheet";
  if (/presentationml|ms-powerpoint/.test(ct) || ["pptx", "ppt"].includes(ext)) return "pptx";
  if (ct === "application/zip" || ct === "application/x-zip-compressed" || ext === "zip") return "zip";
  return null;
}
