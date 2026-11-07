import { useEffect, useRef } from "react";
import { cancelAddAccount, signIn } from "../actions";
import { useStore } from "../store";
import { GoogleG, ConfabMark } from "./icons";

export function SignIn() {
  const status = useStore((s) => s.auth.status);
  const error = useStore((s) => s.auth.error);
  const shell = useStore((s) => s.shell);
  const adding = useStore((s) => !!s.ui.addingFrom);
  const back = useStore((s) => s.accounts.find((a) => a.id === s.ui.addingFrom));
  // First launch in the desktop shell opens the consent page by itself; the button stays for retries.
  const auto = useRef(false);
  useEffect(() => {
    if (shell === "desktop" && status === "out" && !error && !auto.current) { auto.current = true; void signIn(); }
  }, [shell, status, error]);
  return (
    <main className="signin">
      <div className="signin__card">
        <div className="signin__mark"><ConfabMark size={112} animate /></div>
        <h1>{adding ? "Add another account" : "Confab"}</h1>
        <p>{adding ? "Sign in with the other Google account. It gets its own spaces, badge and sign-in, side by side with the first." : "Google Chat, the way a desktop app should feel."}</p>
        <button type="button" className="btn btn--primary btn--google" disabled={status === "loading"} onClick={() => void signIn()}>
          <GoogleG />
          {status === "loading" ? "Waiting for Google…" : "Sign in with Google"}
        </button>
        {adding && <button type="button" className="btn signin__back" onClick={() => void cancelAddAccount()}>Back to {back?.displayName ?? back?.email ?? "the other account"}</button>}
        {error && <div className="signin__err" role="alert">{error}</div>}
        <div className="signin__mode">{shell === "desktop" ? "Your browser opens for consent; this window continues once you are back." : shell === "mock" ? "Fixture mode: no Google, no twin." : "Browser mode: you will be redirected to the twin or Google."}</div>
      </div>
    </main>
  );
}

/** Full-window wait state: the mark draws itself while the label says what is being fetched. */
export function Loader({ label }: { label: string }) {
  return (
    <div className="loading" role="status" aria-live="polite">
      <div className="loading__in">
        <ConfabMark size={72} animate />
        <span>{label}</span>
      </div>
    </div>
  );
}
