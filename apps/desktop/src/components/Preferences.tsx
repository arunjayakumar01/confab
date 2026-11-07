import { forgetPreviewSite, setPrefs, signOut, ui } from "../actions";
import { Icon } from "./icons";
import { useStore, type Prefs } from "../store";
import { Modal } from "./Modal";

export function Preferences({ exiting }: { exiting?: boolean }) {
  const prefs = useStore((s) => s.prefs);
  const user = useStore((s) => s.auth.user);
  const shell = useStore((s) => s.shell);
  const domains = useStore((s) => s.previewDomains);
  const close = () => ui({ prefsOpen: false });
  const themes: { v: Prefs["theme"]; l: string }[] = [{ v: "system", l: "System" }, { v: "light", l: "Light" }, { v: "dark", l: "Dark" }];
  const Switch = ({ k, label }: { k: "reduceTransparency" | "reduceMotion" | "readReceipts" | "sounds"; label: string }) => (
    <button type="button" role="switch" aria-checked={prefs[k]} aria-label={label} className="switch" onClick={() => setPrefs({ [k]: !prefs[k] })} />
  );
  return (
    <Modal title="Preferences" onClose={close} exiting={exiting}>
      <div className="dialog__body">
        <div className="pref">
          <div className="pref__label"><b>Theme</b><span>Follows the OS unless you pick one.</span></div>
          <div className="seg" role="radiogroup" aria-label="Theme">
            {themes.map((t) => (
              <button key={t.v} type="button" role="radio" aria-checked={prefs.theme === t.v} aria-pressed={prefs.theme === t.v} onClick={() => setPrefs({ theme: t.v })}>{t.l}</button>
            ))}
          </div>
        </div>
        <div className="pref">
          <div className="pref__label"><b>Reduce transparency</b><span>Solid panels instead of glass. Also honours the OS setting.</span></div>
          <Switch k="reduceTransparency" label="Reduce transparency" />
        </div>
        <div className="pref">
          <div className="pref__label"><b>Reduce motion</b><span>No slides, pops or glides. Also honours the OS setting.</span></div>
          <Switch k="reduceMotion" label="Reduce motion" />
        </div>
        <div className="pref">
          <div className="pref__label"><b>Notifications</b><span>System notifications for messages that land while you are elsewhere. Never on Do not disturb or in muted spaces.</span></div>
          <div className="seg" role="radiogroup" aria-label="Notifications">
            {([["all", "All"], ["mentions", "DMs & mentions"], ["off", "Off"]] as const).map(([v, l]) => (
              <button key={v} type="button" role="radio" aria-checked={prefs.notifications === v} aria-pressed={prefs.notifications === v} onClick={() => setPrefs({ notifications: v })}>{l}</button>
            ))}
          </div>
        </div>
        <div className="pref">
          <div className="pref__label"><b>Link previews</b><span>Confab fetches the page to build a preview, so the site sees your address. Ask shows a chip per link; sites you allow are remembered.</span></div>
          <div className="seg" role="radiogroup" aria-label="Link previews">
            {([["off", "Off"], ["ask", "Ask"], ["on", "Always"]] as const).map(([v, l]) => (
              <button key={v} type="button" role="radio" aria-checked={prefs.linkPreviews === v} aria-pressed={prefs.linkPreviews === v} onClick={() => setPrefs({ linkPreviews: v })}>{l}</button>
            ))}
          </div>
        </div>
        {domains.length > 0 && (
          <div className="pref pref--list">
            <div className="pref__label"><b>Sites allowed to preview</b><span>{domains.map((d) => <button key={d} type="button" className="chip chip--x" onClick={() => forgetPreviewSite(d)}>{d}<Icon n="x" size={12} /></button>)}</span></div>
          </div>
        )}
        <div className="pref">
          <div className="pref__label"><b>Sounds</b><span>A soft chime when a message lands in a space you are not looking at. Never while you are on Do not disturb.</span></div>
          <Switch k="sounds" label="Sounds" />
        </div>
        <div className="pref">
          <div className="pref__label"><b>Read receipts</b><span>Let others see when you have read their messages. Google still records your read state.</span></div>
          <Switch k="readReceipts" label="Read receipts" />
        </div>
        <div className="pref">
          <div className="pref__label"><b>{user?.displayName ?? "Signed in"}</b><span>{user?.email ?? user?.name} · {shell === "desktop" ? "desktop" : shell === "mock" ? "fixtures" : "browser"} mode</span></div>
          <button type="button" className="btn btn--danger" onClick={() => { close(); void signOut(); }}>Sign out</button>
        </div>
      </div>
    </Modal>
  );
}
