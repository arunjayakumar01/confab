import { useEffect, useRef, useState, type FormEvent } from "react";
import type { DriveItem, DriveWhere } from "../bridge-types";
import {
  closeFeed, connectDrive, driveCopyLink, driveDownload, driveGo, driveLoad, driveMove, driveNewFolder, driveOpen, driveOpenFolder,
  driveParent, driveRename, driveRestore, driveStar, driveTogglePin, driveTrash, driveUi, driveUploadFiles, openExternal,
} from "../actions";
import { getBridge } from "../bridge";
import { FOLDER, humanSize, isFolder, targetId } from "../lib/drive";
import { fileKind } from "../lib/files";
import { stagger } from "../lib/motion";
import { parseTime, relative } from "../lib/time";
import { useStore, type DriveFolder } from "../store";
import { Icon, type IconName } from "./icons";
import { Modal } from "./Modal";
import { Menu } from "./Rail";

const PLACES: { where: DriveWhere; label: string; icon: IconName }[] = [
  { where: { kind: "root" }, label: "My Drive", icon: "drive" },
  { where: { kind: "shared" }, label: "Shared with me", icon: "users" },
  { where: { kind: "drives" }, label: "Shared drives", icon: "hdd" },
  { where: { kind: "starred" }, label: "Starred", icon: "star" },
  { where: { kind: "recent" }, label: "Recent", icon: "clock" },
  { where: { kind: "trash" }, label: "Bin", icon: "trash" },
];
const placeLabel = (w: DriveWhere) => PLACES.find((p) => p.where.kind === w.kind)?.label ?? "Drive";

/** Google Drive in the main pane: places, pinned folders, a folder at a time, and the file actions Drive has. */
export function DriveView() {
  const d = useStore((s) => s.drive);
  const shell = useStore((s) => s.shell);
  const [menu, setMenu] = useState<{ item: DriveItem; x: number; y: number }>();
  const [newOpen, setNewOpen] = useState<{ x: number; y: number }>();
  const fileRef = useRef<HTMLInputElement>(null);
  const searchTimer = useRef(0);
  const parent = driveParent();
  const inFolder = d.where.kind === "folder" ? d.where : undefined;
  const pinnedHere = inFolder && d.pins.some((p) => p.id === inFolder.id);

  const onSearch = (q: string) => {
    driveUi({ query: q });
    window.clearTimeout(searchTimer.current);
    searchTimer.current = window.setTimeout(() => {
      if (q.trim()) driveGo({ kind: "search", query: q.trim() });
      else if (useStore.getState().drive.where.kind === "search") driveGo({ kind: "root" });
    }, 300);
  };
  const openMenu = (item: DriveItem, x: number, y: number) => setMenu({ item, x: Math.min(x, window.innerWidth - 250), y: Math.min(y, window.innerHeight - 380) });

  if (d.connected === false) return <Shell><Connect /></Shell>;

  return (
    <Shell
      head={
        <>
          <div className="drive__search"><Icon n="search" size={14} /><input value={d.query} onChange={(e) => onSearch(e.target.value)} placeholder="Search in Drive" aria-label="Search in Drive" /></div>
          {inFolder && (
            <button type="button" className={`iconbtn${pinnedHere ? " iconbtn--on" : ""}`} aria-pressed={!!pinnedHere} aria-label={pinnedHere ? "Unpin this folder" : "Pin this folder"} title={pinnedHere ? "Unpin this folder" : "Pin this folder"} onClick={() => driveTogglePin(d.trail[d.trail.length - 1] ?? { id: inFolder.id, name: "Folder", driveId: inFolder.driveId })}>
              <Icon n="pin" fill={!!pinnedHere} />
            </button>
          )}
          <button type="button" className="btn btn--primary" disabled={!parent} title={parent ? undefined : "Open My Drive or a folder to add to it"} onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setNewOpen({ x: r.right - 220, y: r.bottom + 6 }); }}>
            <Icon n="plus" />New
          </button>
        </>
      }
    >
      <div className="drive">
        <nav className="drive__nav" aria-label="Drive places">
          {PLACES.map((p) => {
            const on = d.where.kind === p.where.kind && !(d.where.kind === "folder");
            return (
              <button key={p.label} type="button" className={`row${on ? " row--active" : ""}`} aria-current={on ? "page" : undefined} onClick={() => driveGo(p.where)}>
                <span className="row__icon"><Icon n={p.icon} /></span>
                <span className="row__name">{p.label}</span>
              </button>
            );
          })}
          {d.pins.length > 0 && <div className="drive__label">Pinned</div>}
          {d.pins.map((p) => {
            const on = inFolder?.id === p.id;
            return (
              <div key={p.id} className={`row drive__pin${on ? " row--active" : ""}`}>
                <button type="button" className="drive__pin-open" aria-current={on ? "page" : undefined} onClick={() => driveGo({ kind: "folder", id: p.id, driveId: p.driveId }, [p])}>
                  <span className="row__icon"><Icon n="folder" /></span>
                  <span className="row__name">{p.name}</span>
                </button>
                <button type="button" className="iconbtn drive__unpin" aria-label={`Unpin ${p.name}`} title="Unpin" onClick={() => driveTogglePin(p)}><Icon n="x" size={12} /></button>
              </div>
            );
          })}
        </nav>

        <section className="drive__main" aria-label="Files">
          <Crumbs />
          {d.error && <div className="feed__empty"><Icon n="x" size={28} /><b>Drive did not load</b>{d.error}<button type="button" className="btn" onClick={() => void driveLoad()}>Try again</button></div>}
          {d.where.kind === "drives" ? <Drives /> : !d.error && (
            <div className="drive__list" role="grid" aria-label={placeLabel(d.where)} aria-busy={d.loading}>
              <div className="drive__row drive__row--head" role="row">
                <span role="columnheader">Name</span><span role="columnheader">Owner</span><span role="columnheader">Modified</span><span role="columnheader">Size</span><span />
              </div>
              {d.uploading > 0 && <div className="drive__note" role="status">Uploading {d.uploading} {d.uploading === 1 ? "file" : "files"}…</div>}
              {d.items.map((f, i) => <Row key={f.id} f={f} i={i} onMenu={openMenu} />)}
              {d.loading && d.items.length === 0 && <div className="drive__note" role="status">Loading…</div>}
              {!d.loading && d.items.length === 0 && <Empty where={d.where} />}
              {d.nextPageToken && <button type="button" className="btn drive__more" disabled={d.loading} onClick={() => void driveLoad(true)}>{d.loading ? "Loading…" : "Show more"}</button>}
            </div>
          )}
          {parent && shell !== "mock" && <div className="drive__hint">Drop files here to upload them to this folder.</div>}
        </section>
      </div>

      <input ref={fileRef} type="file" multiple hidden onChange={(e) => { void driveUploadFiles(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      {newOpen && (
        <Menu className="menu--drive" label="New" onClose={() => setNewOpen(undefined)} style={{ left: newOpen.x, top: newOpen.y }}>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setNewOpen(undefined); driveUi({ newFolder: true }); }}><Icon n="folder" />New folder</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setNewOpen(undefined); fileRef.current?.click(); }}><Icon n="upload" />Upload files</button>
          <div className="menu__sep" />
          {[["Google Doc", "document"], ["Google Sheet", "spreadsheets"], ["Google Slides", "presentation"]].map(([label, kind]) => (
            <button key={kind} type="button" role="menuitem" className="menu__item" onClick={() => { setNewOpen(undefined); void openExternal(`https://docs.google.com/${kind}/create`); }}><Icon n="external" />{label}</button>
          ))}
        </Menu>
      )}
      {menu && <ItemMenu item={menu.item} x={menu.x} y={menu.y} onClose={() => setMenu(undefined)} />}
      {d.newFolder && <NameDialog title="New folder" initial="Untitled folder" action="Create" onClose={() => driveUi({ newFolder: false })} onSave={(n) => driveNewFolder(n)} />}
      {d.renaming && <NameDialog title="Rename" initial={d.renaming.name} action="Rename" onClose={() => driveUi({ renaming: undefined })} onSave={(n) => driveRename(d.renaming!, n)} />}
      {d.moving && <MoveDialog item={d.moving} onClose={() => driveUi({ moving: undefined })} />}
    </Shell>
  );
}

function Shell({ head, children }: { head?: React.ReactNode; children: React.ReactNode }) {
  const d = useStore((s) => s.drive);
  const crumb = d.where.kind === "folder" ? d.trail[d.trail.length - 1]?.name : d.where.kind === "search" ? `Results for "${d.where.query}"` : placeLabel(d.where);
  return (
    <>
      <div className="header glass-strong">
        <div className="header__name"><Icon n="drive" /><span className="header__name-in">Drive</span></div>
        {d.connected && <div className="header__meta"><span className="header__desc">{d.loading ? "Loading…" : crumb}</span></div>}
        <div className="header__spacer" />
        {head}
        <button type="button" className="iconbtn" aria-label="Close" onClick={closeFeed}><Icon n="x" /></button>
      </div>
      <div className="feed feed--drive">{children}</div>
    </>
  );
}

/** First visit: Drive needs one more permission. The chat sign-in is kept; Google adds Drive to it. */
function Connect() {
  const { connecting, error } = useStore((s) => s.drive);
  return (
    <div className="feed__empty drive__connect">
      <Icon n="drive" size={36} />
      <b>Open your Google Drive here</b>
      <span>Confab needs permission to see and manage your Drive files. Your browser opens once to ask. You stay signed in to chat.</span>
      <button type="button" className="btn btn--primary" disabled={connecting} onClick={() => void connectDrive()}>{connecting ? "Waiting for Google…" : "Allow Drive access"}</button>
      {error && <div className="dialog__note dialog__note--err" role="alert">{error}</div>}
    </div>
  );
}

function Crumbs() {
  const { where, trail, drives } = useStore((s) => s.drive);
  if (where.kind !== "folder") return null;
  const top = where.driveId ? drives.find((x) => x.id === where.driveId) : undefined;
  return (
    <nav className="drive__crumbs" aria-label="Folder path">
      <button type="button" className="drive__crumb" onClick={() => driveGo(where.driveId ? { kind: "drives" } : { kind: "root" })}>{where.driveId ? "Shared drives" : "My Drive"}</button>
      {top && trail[0]?.id !== top.id && <><Icon n="right" size={12} /><button type="button" className="drive__crumb" onClick={() => driveGo({ kind: "folder", id: top.id, driveId: top.id }, [{ id: top.id, name: top.name, driveId: top.id }])}>{top.name}</button></>}
      {trail.map((t, i) => (
        <span key={t.id} className="drive__crumb-wrap">
          <Icon n="right" size={12} />
          {i === trail.length - 1 ? <span className="drive__crumb drive__crumb--here" aria-current="page">{t.name}</span> : <button type="button" className="drive__crumb" onClick={() => driveOpenFolder(t)}>{t.name}</button>}
        </span>
      ))}
    </nav>
  );
}

function Drives() {
  const drives = useStore((s) => s.drive.drives);
  if (!drives.length) return <div className="feed__empty"><Icon n="hdd" size={28} /><b>No shared drives</b>Shared drives your organisation adds you to show up here.</div>;
  return (
    <div className="drive__list" role="list">
      {drives.map((x, i) => (
        <button key={x.id} type="button" role="listitem" className="drive__row drive__row--drive" style={stagger(i, 16)} onClick={() => driveGo({ kind: "folder", id: x.id, driveId: x.id }, [{ id: x.id, name: x.name, driveId: x.id }])}>
          <span className="drive__name"><span className="drive__kind drive__kind--folder"><Icon n="hdd" size={14} /></span><span className="drive__title">{x.name}</span></span>
        </button>
      ))}
    </div>
  );
}

function Empty({ where }: { where: DriveWhere }) {
  const text: Record<string, [string, string]> = {
    root: ["Your Drive is empty", "Use New to make a folder or upload files, or drop files here."],
    folder: ["This folder is empty", "Use New to add a folder or upload files, or drop files here."],
    shared: ["Nothing shared with you yet", "Files people share with you appear here."],
    starred: ["Nothing starred", "Star files and folders to find them quickly."],
    recent: ["No recent files", "Files you change show up here."],
    trash: ["The bin is empty", "Items you move to the bin wait here until Google deletes them after 30 days."],
    search: ["No matching files", "Try another word, or search in Google Drive on the web."],
  };
  const [b, t] = text[where.kind] ?? ["Nothing here", ""];
  return <div className="feed__empty"><Icon n="folder" size={28} /><b>{b}</b>{t}</div>;
}

function Row({ f, i, onMenu }: { f: DriveItem; i: number; onMenu: (f: DriveItem, x: number, y: number) => void }) {
  const folder = isFolder(f);
  const k = fileKind(folder ? FOLDER : f.mimeType, f.name);
  const owner = f.owners?.[0];
  return (
    <div className="drive__row" role="row" style={stagger(i, 12)} onContextMenu={(e) => { e.preventDefault(); onMenu(f, e.clientX, e.clientY); }}>
      <button type="button" className="drive__name" onClick={() => void driveOpen(f)} onKeyDown={(e) => { if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) { e.preventDefault(); const r = e.currentTarget.getBoundingClientRect(); onMenu(f, r.left + 40, r.bottom); } }} title={f.name}>
        {folder ? <span className="drive__kind drive__kind--folder"><Icon n="folder" size={14} /></span> : <span className="att__kind drive__kind" style={{ ["--k" as string]: k.color }}>{k.label}</span>}
        <span className="drive__title">{f.name}</span>
        {f.starred && <span className="drive__star" aria-label="Starred"><Icon n="star" size={12} fill /></span>}
        {f.shared && <span className="drive__shared" aria-label="Shared" title="Shared"><Icon n="users" size={12} /></span>}
      </button>
      <span className="drive__cell">{f.driveId && !owner ? "Shared drive" : owner?.me ? "me" : owner?.displayName ?? ""}</span>
      <span className="drive__cell">{f.modifiedTime ? relative(parseTime(f.modifiedTime)) : ""}</span>
      <span className="drive__cell">{folder ? "" : humanSize(f.size)}</span>
      <button type="button" className="iconbtn drive__more-btn" aria-label={`Actions for ${f.name}`} aria-haspopup="menu" onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); onMenu(f, r.right - 240, r.bottom + 4); }}><Icon n="more" /></button>
    </div>
  );
}

function ItemMenu({ item, x, y, onClose }: { item: DriveItem; x: number; y: number; onClose: () => void }) {
  const pins = useStore((s) => s.drive.pins);
  const folder = isFolder(item);
  const can = item.capabilities ?? {};
  const go = (fn: () => void) => () => { onClose(); fn(); };
  const pinned = pins.some((p) => p.id === targetId(item));
  if (item.trashed) {
    return (
      <Menu className="menu--drive" label={item.name} onClose={onClose} style={{ left: x, top: y }}>
        <div className="menu__title">{item.name}</div>
        <button type="button" role="menuitem" className="menu__item" disabled={can.canTrash === false} onClick={go(() => void driveRestore(item))}><Icon n="restore" />Restore</button>
        <div className="menu__note">Google deletes items in the bin for good after 30 days.</div>
      </Menu>
    );
  }
  return (
    <Menu className="menu--drive" label={item.name} onClose={onClose} style={{ left: x, top: y }}>
      <div className="menu__title">{item.name}</div>
      <button type="button" role="menuitem" className="menu__item" onClick={go(() => void driveOpen(item))}><Icon n={folder ? "folder" : "eye"} />{folder ? "Open" : "Preview"}</button>
      {item.webViewLink && <button type="button" role="menuitem" className="menu__item" onClick={go(() => void openExternal(item.webViewLink!))}><Icon n="external" />Open in Google Drive</button>}
      {!folder && can.canDownload !== false && <button type="button" role="menuitem" className="menu__item" onClick={go(() => void driveDownload(item))}><Icon n="download" />Download</button>}
      {item.webViewLink && <button type="button" role="menuitem" className="menu__item" onClick={go(() => void driveCopyLink(item))}><Icon n="link" />Copy link</button>}
      {item.webViewLink && <button type="button" role="menuitem" className="menu__item" onClick={go(() => void openExternal(item.webViewLink!))}><Icon n="users" />Share in Google Drive</button>}
      <div className="menu__sep" />
      {folder && <button type="button" role="menuitem" className="menu__item" onClick={go(() => driveTogglePin({ id: targetId(item), name: item.name, driveId: item.driveId }))}><Icon n="pin" />{pinned ? "Unpin" : "Pin to the side"}</button>}
      <button type="button" role="menuitem" className="menu__item" onClick={go(() => void driveStar(item))}><Icon n="star" />{item.starred ? "Remove from Starred" : "Add to Starred"}</button>
      {can.canRename !== false && <button type="button" role="menuitem" className="menu__item" onClick={go(() => driveUi({ renaming: item }))}><Icon n="edit" />Rename</button>}
      {can.canEdit !== false && <button type="button" role="menuitem" className="menu__item" onClick={go(() => driveUi({ moving: item }))}><Icon n="move" />Move</button>}
      {can.canTrash !== false && <><div className="menu__sep" /><button type="button" role="menuitem" className="menu__item menu__item--danger" onClick={go(() => void driveTrash(item))}><Icon n="trash" />Move to bin</button></>}
    </Menu>
  );
}

function NameDialog({ title, initial, action, onClose, onSave }: { title: string; initial: string; action: string; onClose: () => void; onSave: (name: string) => Promise<void> }) {
  const [name, setName] = useState(initial);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    // Select the name without its extension, as Drive does.
    const dot = initial.lastIndexOf(".");
    el.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);
  const go = (e: FormEvent) => {
    e.preventDefault();
    const n = name.trim();
    if (!n || busy) return;
    setBusy(true);
    void onSave(n).finally(onClose);
  };
  return (
    <Modal title={title} onClose={onClose}>
      <form onSubmit={go}>
        <div className="dialog__body"><label className="field"><span>Name</span><input ref={ref} value={name} onChange={(e) => setName(e.target.value)} /></label></div>
        <div className="dialog__actions">
          <button type="button" className="btn" onClick={onClose} disabled={busy}>Cancel</button>
          <button type="submit" className="btn btn--primary" disabled={busy || !name.trim()}>{busy ? "Saving…" : action}</button>
        </div>
      </form>
    </Modal>
  );
}

/** Pick a destination folder by browsing My Drive and shared drives. */
function MoveDialog({ item, onClose }: { item: DriveItem; onClose: () => void }) {
  const drives = useStore((s) => s.drive.drives);
  const [path, setPath] = useState<DriveFolder[]>([]);
  const [folders, setFolders] = useState<DriveItem[]>();
  const [busy, setBusy] = useState(false);
  const here = path[path.length - 1];
  useEffect(() => {
    let live = true;
    setFolders(undefined);
    const where: DriveWhere = here ? { kind: "folder", id: here.id, driveId: here.driveId } : { kind: "root" };
    void getBridge().then((b) => b.driveList(where)).then((p) => { if (live) setFolders(p.files.filter((f) => f.mimeType === FOLDER && f.id !== item.id)); }).catch(() => { if (live) setFolders([]); });
    return () => { live = false; };
  }, [here, item.id]);
  const dest: DriveFolder = here ?? { id: "root", name: "My Drive" };
  const already = item.parents?.includes(dest.id);
  const move = () => { setBusy(true); void driveMove(item, dest).finally(onClose); };
  return (
    <Modal title={`Move ${item.name}`} onClose={onClose}>
      <div className="dialog__body">
        <nav className="drive__crumbs drive__crumbs--dialog" aria-label="Destination">
          <button type="button" className="drive__crumb" onClick={() => setPath([])}>My Drive</button>
          {path.map((p, i) => <span key={p.id} className="drive__crumb-wrap"><Icon n="right" size={12} /><button type="button" className="drive__crumb" onClick={() => setPath(path.slice(0, i + 1))}>{p.name}</button></span>)}
        </nav>
        <div className="drive__picker" role="listbox" aria-label="Folders">
          {folders === undefined && <div className="drive__note">Loading…</div>}
          {folders?.length === 0 && <div className="drive__note">No folders inside. Move it here, or go back.</div>}
          {folders?.map((f) => (
            <button key={f.id} type="button" role="option" aria-selected={false} className="drive__pick" onClick={() => setPath([...path, { id: f.id, name: f.name, driveId: f.driveId }])}>
              <Icon n="folder" size={14} />{f.name}<Icon n="right" size={12} />
            </button>
          ))}
          {path.length === 0 && drives.map((x) => (
            <button key={x.id} type="button" role="option" aria-selected={false} className="drive__pick" onClick={() => setPath([{ id: x.id, name: x.name, driveId: x.id }])}>
              <Icon n="hdd" size={14} />{x.name}<Icon n="right" size={12} />
            </button>
          ))}
        </div>
      </div>
      <div className="dialog__actions">
        <button type="button" className="btn" onClick={onClose} disabled={busy}>Cancel</button>
        <button type="button" className="btn btn--primary" disabled={busy || already} onClick={move}>{busy ? "Moving…" : already ? "Already here" : `Move to ${dest.name}`}</button>
      </div>
    </Modal>
  );
}
