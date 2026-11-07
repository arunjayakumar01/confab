import { useEffect, useMemo, useState } from "react";
import type { Message } from "../bridge-types";
import { allowPreview, canPreview, loadPreview, openExternal, previewAllowed } from "../actions";
import { linksOf } from "../lib/markdown";
import { useExit, useLast } from "../lib/motion";
import { bodyOf, useStore } from "../store";
import { Icon } from "./icons";
import { Menu, menuAt } from "./Rail";

const hostOf = (url: string): string => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; } };

/**
 * Previews for a message's links, consent first: a chip per link until the user says yes for that link, its
 * site or every link (Preferences can also turn them off or on). Hidden previews stay hidden for the session.
 */
export function LinkPreviews({ message }: { message: Message }) {
  const links = useMemo(() => linksOf(bodyOf(message)), [message.text, message.formattedText]);
  const mode = useStore((s) => s.prefs.linkPreviews);
  const domains = useStore((s) => s.previewDomains);
  const [hidden, setHidden] = useState<string[]>([]);
  if (!canPreview() || mode === "off" || links.length === 0) return null;
  const shown = links.filter((l) => !hidden.includes(l));
  if (shown.length === 0) return null;
  return (
    <div className="previews">
      {shown.map((url) => <One key={url} url={url} allowed={previewAllowed(url) || domains.includes(hostOf(url))} onHide={() => setHidden((h) => [...h, url])} />)}
    </div>
  );
}

function One({ url, allowed, onHide }: { url: string; allowed: boolean; onHide: () => void }) {
  const p = useStore((s) => s.previews[url]);
  const [ask, setAsk] = useState<React.CSSProperties | null>(null);
  const menu = useExit(!!ask, 120);
  const style = useLast(ask);
  const host = hostOf(url);
  useEffect(() => { if (allowed && p === undefined) void loadPreview(url); }, [allowed, p, url]);
  if (!allowed && p === undefined) {
    return (
      <>
        <button type="button" className="preview__ask" onClick={(e) => setAsk(menuAt(e, 272, 190))} aria-haspopup="menu" aria-expanded={!!ask}>
          <Icon n="globe" />
          Preview {host}
          <span>· ask</span>
        </button>
        {menu.mounted && style && (
          <Menu className="menu--ctx" label="Show a preview" style={style} exiting={menu.exiting} onClose={() => setAsk(null)}>
            <div className="menu__note">Confab fetches the page to build the preview, so {host} sees your address.</div>
            <button type="button" role="menuitem" className="menu__item" onClick={() => { setAsk(null); allowPreview(url, "once"); }}><Icon n="eye" />Show this preview</button>
            <button type="button" role="menuitem" className="menu__item" onClick={() => { setAsk(null); allowPreview(url, "site"); }}><Icon n="check" />Always for {host}</button>
            <button type="button" role="menuitem" className="menu__item" onClick={() => { setAsk(null); allowPreview(url, "always"); }}><Icon n="checkall" />Always, for every link</button>
            <div className="menu__sep" />
            <button type="button" role="menuitem" className="menu__item" onClick={() => { setAsk(null); onHide(); }}><Icon n="x" />Not now</button>
          </Menu>
        )}
      </>
    );
  }
  if (p === undefined || (p && "loading" in p)) return <div className="preview preview--loading">Fetching {host}…</div>;
  if (p === null) return null;
  if ("error" in p) return <div className="preview__err">No preview for {host}: {p.error}.</div>;
  return (
    <div className="preview" role="group" aria-label={`Preview of ${host}`}>
      <button type="button" className="preview__body" onClick={() => void openExternal(url)} title={url}>
        <span className="preview__site">{p.site ?? host}</span>
        {p.title && <span className="preview__title">{p.title}</span>}
        {p.description && <span className="preview__desc">{p.description}</span>}
      </button>
      {p.image && <img className="preview__img" src={p.image} alt="" loading="lazy" onError={(e) => { e.currentTarget.style.display = "none"; }} />}
      <button type="button" className="iconbtn preview__x" aria-label="Hide preview" title="Hide preview" onClick={onHide}><Icon n="x" /></button>
    </div>
  );
}
