import { useEffect, useMemo, useRef, useState } from "react";
import { openExternal, openViewer, toast } from "../actions";
import { fileKind, previewKind, type PreviewKind } from "../lib/files";
import { Icon } from "./icons";

const BYTES_CAP = 60 * 1024 * 1024;

/** Fetches the file once (shell proxy, data: or blob: URL) and hands the bytes to a viewer. */
export function useBytes(src: string): { bytes?: ArrayBuffer; err?: string } {
  const [state, setState] = useState<{ bytes?: ArrayBuffer; err?: string }>({});
  useEffect(() => {
    let live = true;
    setState({});
    fetch(src)
      .then(async (r) => {
        if (!r.ok) throw new Error(`the shell answered ${r.status}`);
        const len = Number(r.headers.get("content-length") ?? 0);
        if (len > BYTES_CAP) throw new Error("larger than 60 MB");
        const b = await r.arrayBuffer();
        if (b.byteLength > BYTES_CAP) throw new Error("larger than 60 MB");
        return b;
      })
      .then((bytes) => { if (live) setState({ bytes }); })
      .catch((e: unknown) => { if (live) setState({ err: String((e as Error)?.message ?? e) }); });
    return () => { live = false; };
  }, [src]);
  return state;
}

export const fmtSize = (n?: number): string => (n === undefined ? "" : n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(0)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

function Note({ title, body, children }: { title: string; body?: string; children?: React.ReactNode }) {
  return <div className="viewer__none"><Icon n="file" size={28} /><b>{title}</b>{body}{children}</div>;
}
function Spinner({ label }: { label: string }) {
  return <div className="viewer__none viewer__none--busy"><span className="spinner" />{label}</div>;
}

/** Picks the viewer for a kind; `onSize` reports the byte size for the bar. */
export function ViewerBody({ kind, src, name, mime, onSize }: { kind: PreviewKind | null; src: string; name: string; mime?: string; onSize: (n: number) => void }) {
  if (kind === "image") return <img className="viewer__img" src={src} alt={name} />;
  if (kind === "video") return <video className="viewer__media" src={src} controls autoPlay />;
  if (kind === "audio") return <div className="viewer__audio"><Icon n="file" size={40} /><audio src={src} controls autoPlay /></div>;
  if (kind === "pdf") return <PdfView src={src} onSize={onSize} />;
  if (kind === "docx") return <DocxView src={src} onSize={onSize} />;
  if (kind === "sheet") return <SheetView src={src} name={name} onSize={onSize} />;
  if (kind === "zip") return <ZipView src={src} name={name} onSize={onSize} />;
  if (kind === "text") return <TextView src={src} onSize={onSize} />;
  if (kind === "pptx") return <Note title="No preview for presentations yet" body="Save it and open it in Keynote or PowerPoint." />;
  return <Note title={`No preview for ${fileKind(mime, name).name}`} body="Save it, or open it in the browser." />;
}

// ---- text ----
function TextView({ src, onSize }: { src: string; onSize: (n: number) => void }) {
  const { bytes, err } = useBytes(src);
  const text = useMemo(() => (bytes ? new TextDecoder().decode(bytes) : ""), [bytes]);
  useEffect(() => { if (bytes) onSize(bytes.byteLength); }, [bytes, onSize]);
  if (err) return <Note title="Could not load this file" body={err} />;
  if (!bytes) return <Spinner label="Loading…" />;
  const lines = text.split("\n");
  return (
    <div className="paper paper--mono">
      <ol className="code">
        {lines.slice(0, 5000).map((l, i) => <li key={i}>{l}</li>)}
      </ol>
      {lines.length > 5000 && <div className="paper__more">Showing the first 5,000 lines of {lines.length.toLocaleString()}.</div>}
    </div>
  );
}

// ---- pdf (PDF.js pages on canvases, rendered as they scroll into view) ----
type PdfDoc = { numPages: number; getPage: (n: number) => Promise<PdfPage> };
type PdfPage = { getViewport: (o: { scale: number }) => { width: number; height: number }; render: (o: { canvasContext: CanvasRenderingContext2D; viewport: { width: number; height: number } }) => { promise: Promise<void> } };
let pdfLib: Promise<typeof import("pdfjs-dist")> | null = null;
/**
 * PDF.js on the main thread: the worker module is imported here and handed to PDF.js as `globalThis.pdfjsWorker`,
 * which makes it skip the Web Worker. In the Tauri webview the worker spins up but never answers, so a document
 * stayed on "Opening" forever; on the main thread a few MB parse in well under a second.
 * The legacy build, because the webview is the OS's WebKit: macOS 15.0 lacks features the modern build calls.
 */
async function loadPdfJs() {
  if (!pdfLib) {
    pdfLib = Promise.all([import("pdfjs-dist/legacy/build/pdf.mjs"), import("pdfjs-dist/legacy/build/pdf.worker.min.mjs")]).then(([lib, worker]) => {
      (globalThis as unknown as { pdfjsWorker: unknown }).pdfjsWorker = worker;
      lib.GlobalWorkerOptions.workerSrc = "main-thread";
      return lib;
    }, (e: unknown) => { pdfLib = null; throw e; });
  }
  return pdfLib;
}
const OPEN_MS = 20_000;
function PdfView({ src, onSize }: { src: string; onSize: (n: number) => void }) {
  const { bytes, err } = useBytes(src);
  const [doc, setDoc] = useState<PdfDoc | null>(null);
  const [fail, setFail] = useState("");
  const [scale, setScale] = useState(1.2);
  const [fit, setFit] = useState(1.2); // what 100% means: the page's width in the room there is, never above the old default
  const [page, setPage] = useState(1);
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => { if (bytes) onSize(bytes.byteLength); }, [bytes, onSize]);
  useEffect(() => {
    if (!bytes) return;
    let live = true;
    setDoc(null);
    setFail("");
    const timeout = new Promise<never>((_, rej) => window.setTimeout(() => rej(new Error("it took longer than 20 seconds to parse")), OPEN_MS));
    Promise.race([loadPdfJs().then((lib) => lib.getDocument({ data: new Uint8Array(bytes.slice(0)) }).promise), timeout])
      .then((d) => { if (live) setDoc(d as unknown as PdfDoc); })
      .catch((e: unknown) => { if (live) setFail(String((e as Error)?.message ?? e)); });
    return () => { live = false; };
  }, [bytes]);
  useEffect(() => {
    if (!doc) return;
    let live = true;
    void doc.getPage(1).then((p) => {
      const room = (scroller.current?.clientWidth ?? 0) - 32;
      if (!live || room <= 0) return;
      const f = Math.min(1.2, room / p.getViewport({ scale: 1 }).width);
      setFit(f);
      setScale(f);
    });
    return () => { live = false; };
  }, [doc]);
  // The page counter follows the page nearest the top of the scroller.
  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    const pages = [...el.querySelectorAll<HTMLElement>("[data-page]")];
    const top = el.scrollTop + 40;
    let cur = 1;
    for (const p of pages) if (p.offsetTop <= top) cur = Number(p.dataset.page);
    setPage(cur);
  };
  const go = (n: number) => {
    const el = scroller.current?.querySelector<HTMLElement>(`[data-page="${n}"]`);
    el?.scrollIntoView({ block: "start", behavior: "smooth" });
  };
  if (err) return <Note title="Could not load this PDF" body={err} />;
  if (fail) return <Note title="This PDF did not open" body={fail} />;
  if (!bytes || !doc) return <Spinner label="Opening the PDF…" />;
  return (
    <div className="docview">
      <div className="docview__tools" role="toolbar" aria-label="PDF controls">
        <button type="button" className="iconbtn" aria-label="Previous page" disabled={page <= 1} onClick={() => go(page - 1)}><Icon n="left" /></button>
        <span className="docview__page">Page {page} of {doc.numPages}</span>
        <button type="button" className="iconbtn" aria-label="Next page" disabled={page >= doc.numPages} onClick={() => go(page + 1)}><Icon n="right" /></button>
        <span className="sep" />
        <button type="button" className="iconbtn" aria-label="Zoom out" onClick={() => setScale((s) => Math.max(0.5, Math.round((s - 0.2) * 10) / 10))}><Icon n="minus" /></button>
        <button type="button" className="docview__zoom" onClick={() => setScale(fit)} title="Reset zoom">{Math.round(scale / fit * 100)}%</button>
        <button type="button" className="iconbtn" aria-label="Zoom in" onClick={() => setScale((s) => Math.min(3, Math.round((s + 0.2) * 10) / 10))}><Icon n="plus" /></button>
      </div>
      <div className="docview__scroll" ref={scroller} onScroll={onScroll}>
        {Array.from({ length: doc.numPages }, (_, i) => <PdfPageView key={i} doc={doc} n={i + 1} scale={scale} />)}
      </div>
    </div>
  );
}
function PdfPageView({ doc, n, scale }: { doc: PdfDoc; n: number; scale: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [seen, setSeen] = useState(n <= 2);
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || seen || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) { setSeen(true); io.disconnect(); } }, { rootMargin: "600px" });
    io.observe(el);
    return () => io.disconnect();
  }, [seen]);
  useEffect(() => {
    if (!seen) return;
    let live = true;
    void doc.getPage(n).then(async (p) => {
      const canvas = ref.current;
      if (!canvas || !live) return;
      const dpr = window.devicePixelRatio || 1;
      const vp = p.getViewport({ scale });
      canvas.width = Math.floor(vp.width * dpr);
      canvas.height = Math.floor(vp.height * dpr);
      canvas.style.width = `${Math.floor(vp.width)}px`;
      canvas.style.height = `${Math.floor(vp.height)}px`;
      setSize({ w: Math.floor(vp.width), h: Math.floor(vp.height) });
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      await p.render({ canvasContext: ctx, viewport: vp }).promise;
    });
    return () => { live = false; };
  }, [doc, n, scale, seen]);
  return <canvas ref={ref} className="docview__canvas" data-page={n} style={size ? { width: size.w, height: size.h } : { width: 612 * scale, height: 792 * scale }} />;
}

// ---- docx (mammoth to HTML on a paper sheet) ----
function DocxView({ src, onSize }: { src: string; onSize: (n: number) => void }) {
  const { bytes, err } = useBytes(src);
  const [html, setHtml] = useState<string | null>(null);
  const [fail, setFail] = useState("");
  const [notes, setNotes] = useState(0);
  useEffect(() => { if (bytes) onSize(bytes.byteLength); }, [bytes, onSize]);
  useEffect(() => {
    if (!bytes) return;
    let live = true;
    setHtml(null);
    import("mammoth")
      .then((m) => m.default.convertToHtml({ arrayBuffer: bytes }))
      .then((r) => { if (live) { setHtml(scrub(r.value)); setNotes(r.messages.length); } })
      .catch((e: unknown) => live && setFail(String((e as Error)?.message ?? e)));
    return () => { live = false; };
  }, [bytes]);
  if (err) return <Note title="Could not load this document" body={err} />;
  if (fail) return <Note title="This document did not open" body={fail} />;
  if (!bytes || html === null) return <Spinner label="Opening the document…" />;
  return (
    <div className="paper paper--doc" onClick={(e) => { const a = (e.target as HTMLElement).closest("a"); if (a?.href) { e.preventDefault(); void openExternal(a.href); } }}>
      {html ? <div className="doc" dangerouslySetInnerHTML={{ __html: html }} /> : <Note title="An empty document" />}
      {notes > 0 && <div className="paper__more">Some formatting in this file has no equivalent here; the text is complete.</div>}
    </div>
  );
}
/** mammoth emits plain HTML, but a shared file is still someone else's input: no scripts, no script links. */
function scrub(html: string): string {
  return html.replace(/<script[\s\S]*?<\/script>/gi, "").replace(/\son\w+="[^"]*"/gi, "").replace(/href="javascript:[^"]*"/gi, 'href="#"');
}

// ---- sheets (SheetJS, one table per sheet, tabs) ----
type Grid = (string | number | boolean | null)[][];
function SheetView({ src, name, onSize }: { src: string; name: string; onSize: (n: number) => void }) {
  const { bytes, err } = useBytes(src);
  const [sheets, setSheets] = useState<{ name: string; rows: Grid }[] | null>(null);
  const [fail, setFail] = useState("");
  const [tab, setTab] = useState(0);
  useEffect(() => { if (bytes) onSize(bytes.byteLength); }, [bytes, onSize]);
  useEffect(() => {
    if (!bytes) return;
    let live = true;
    setSheets(null);
    import("xlsx")
      .then((X) => {
        const wb = X.read(new Uint8Array(bytes), { type: "array", cellDates: true });
        return wb.SheetNames.map((n) => ({ name: n, rows: (X.utils.sheet_to_json(wb.Sheets[n]!, { header: 1, raw: false, defval: null }) as Grid).slice(0, 1000) }));
      })
      .then((s) => live && setSheets(s))
      .catch((e: unknown) => live && setFail(String((e as Error)?.message ?? e)));
    return () => { live = false; };
  }, [bytes, name]);
  if (err) return <Note title="Could not load this spreadsheet" body={err} />;
  if (fail) return <Note title="This spreadsheet did not open" body={fail} />;
  if (!bytes || !sheets) return <Spinner label="Opening the spreadsheet…" />;
  const cur = sheets[Math.min(tab, sheets.length - 1)];
  if (!cur) return <Note title="An empty workbook" />;
  const cols = Math.min(60, cur.rows.reduce((a, r) => Math.max(a, r.length), 0));
  const letters = (i: number) => (i < 26 ? String.fromCharCode(65 + i) : String.fromCharCode(64 + Math.floor(i / 26)) + String.fromCharCode(65 + (i % 26)));
  return (
    <div className="docview">
      {sheets.length > 1 && (
        <div className="docview__tabs" role="tablist">
          {sheets.map((s, i) => <button key={s.name} type="button" role="tab" aria-selected={i === tab} className={`docview__tab${i === tab ? " docview__tab--on" : ""}`} onClick={() => setTab(i)}>{s.name}</button>)}
        </div>
      )}
      <div className="docview__scroll docview__scroll--grid">
        <table className="grid">
          <thead>
            <tr><th className="grid__corner" />{Array.from({ length: cols }, (_, i) => <th key={i}>{letters(i)}</th>)}</tr>
          </thead>
          <tbody>
            {cur.rows.map((r, i) => (
              <tr key={i}>
                <th>{i + 1}</th>
                {Array.from({ length: cols }, (_, j) => <td key={j} className={typeof r[j] === "number" ? "grid__num" : undefined}>{r[j] === null || r[j] === undefined ? "" : String(r[j])}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
        {cur.rows.length >= 1000 && <div className="paper__more">Showing the first 1,000 rows.</div>}
      </div>
    </div>
  );
}

// ---- zip (entries you can open in place) ----
interface Entry { path: string; name: string; depth: number; size: number; dir: boolean }
function ZipView({ src, name, onSize }: { src: string; name: string; onSize: (n: number) => void }) {
  const { bytes, err } = useBytes(src);
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [fail, setFail] = useState("");
  const [busy, setBusy] = useState("");
  useEffect(() => { if (bytes) onSize(bytes.byteLength); }, [bytes, onSize]);
  useEffect(() => {
    if (!bytes) return;
    let live = true;
    setEntries(null);
    import("fflate")
      .then((F) => {
        // Sizes only: the filter sees every entry and inflates none until one is opened.
        const list: Entry[] = [];
        const seenDirs = new Set<string>();
        F.unzipSync(new Uint8Array(bytes), {
          filter: (f) => {
            const parts = f.name.split("/").filter(Boolean);
            for (let i = 1; i < parts.length; i++) {
              const d = parts.slice(0, i).join("/") + "/";
              if (!seenDirs.has(d)) { seenDirs.add(d); list.push({ path: d, name: parts[i - 1]!, depth: i - 1, size: 0, dir: true }); }
            }
            if (!f.name.endsWith("/")) list.push({ path: f.name, name: parts[parts.length - 1] ?? f.name, depth: parts.length - 1, size: f.originalSize, dir: false });
            return false;
          },
        });
        return list.sort((a, b) => a.path.localeCompare(b.path));
      })
      .then((l) => live && setEntries(l))
      .catch((e: unknown) => live && setFail(String((e as Error)?.message ?? e)));
    return () => { live = false; };
  }, [bytes]);
  const open = async (e: Entry) => {
    if (!bytes || e.dir) return;
    if (e.size > 40 * 1024 * 1024) { toast("That entry is larger than 40 MB"); return; }
    setBusy(e.path);
    try {
      const F = await import("fflate");
      const out = F.unzipSync(new Uint8Array(bytes), { filter: (f) => f.name === e.path });
      const data = out[e.path];
      if (!data) throw new Error("entry not found");
      const mime = mimeFor(e.name);
      const url = URL.createObjectURL(new Blob([data], { type: mime }));
      openViewer(url, e.name, mime, undefined, { src, name, mime: "application/zip" });
    } catch (x) {
      toast(`Could not open ${e.name}: ${String((x as Error)?.message ?? x)}`);
    } finally {
      setBusy("");
    }
  };
  if (err) return <Note title="Could not load this archive" body={err} />;
  if (fail) return <Note title="This archive did not open" body={fail} />;
  if (!bytes || !entries) return <Spinner label="Reading the archive…" />;
  const files = entries.filter((e) => !e.dir);
  return (
    <div className="paper paper--list">
      <div className="ziplist__head">{files.length} {files.length === 1 ? "file" : "files"}, {fmtSize(files.reduce((a, e) => a + e.size, 0))} unpacked</div>
      <div className="ziplist" role="list">
        {entries.map((e) => {
          const k = e.dir ? null : fileKind(mimeFor(e.name), e.name);
          const can = !e.dir && !!previewKind(mimeFor(e.name), e.name);
          return (
            <button key={e.path} type="button" role="listitem" className={`zipentry${e.dir ? " zipentry--dir" : ""}${can ? "" : " zipentry--plain"}`} style={{ paddingLeft: 12 + e.depth * 18 }} disabled={e.dir || !can} onClick={() => void open(e)} title={can ? `Open ${e.name}` : e.dir ? undefined : "No viewer for this type"}>
              {e.dir ? <span className="zipentry__dir"><Icon n="right" size={12} /></span> : <span className="att__kind zipentry__kind" style={{ ["--k" as string]: k!.color }}>{k!.label}</span>}
              <span className="zipentry__name">{e.name}</span>
              {!e.dir && <span className="zipentry__size">{busy === e.path ? "opening…" : fmtSize(e.size)}</span>}
              {can && <Icon n="eye" size={13} />}
            </button>
          );
        })}
      </div>
    </div>
  );
}
const MIME: Record<string, string> = { pdf: "application/pdf", png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", svg: "image/svg+xml", mp4: "video/mp4", mov: "video/quicktime", mp3: "audio/mpeg", wav: "audio/wav", m4a: "audio/mp4", txt: "text/plain", md: "text/markdown", csv: "text/csv", json: "application/json", docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation", zip: "application/zip" };
export const mimeFor = (name: string): string => MIME[(name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "").toLowerCase()] ?? "application/octet-stream";
