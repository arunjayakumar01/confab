import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./icons";

/**
 * A centred dialog over a dimmed backdrop. Esc, the X and a backdrop click call `onClose`; the host keeps it
 * mounted through `useExit` and passes `exiting` so the close plays out before unmount.
 */
export function Modal({ title, onClose, children, center, bare, exiting }: { title?: string; onClose: () => void; children: ReactNode; center?: boolean; bare?: boolean; exiting?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  // Portal: a backdrop-filter ancestor (the glass sidebar) would otherwise turn the fixed overlay into a sidebar-sized box.
  return createPortal(
    <div className={`overlay${center ? " overlay--center" : ""}${exiting ? " overlay--out" : ""}`} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" role="dialog" aria-modal="true" aria-label={title}>
        {title && !bare && (
          <div className="dialog__head">
            {title}
            <button type="button" className="iconbtn" aria-label="Close" onClick={onClose}>
              <Icon n="x" />
            </button>
          </div>
        )}
        {children}
      </div>
    </div>,
    document.body,
  );
}
