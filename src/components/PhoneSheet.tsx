// PhoneSheet — the chrome every iOS bottom sheet shares: scrim, grab handle,
// head, and the scrolling body. Library, Tabs and Settings all mount it, so
// the dismiss target sits in the same place in all three and a change to the
// sheet (grab handle, close glyph, swipe-to-dismiss) reaches them together.
//
// The scrim belongs to the sheet, not to the shell around it: exactly one
// sheet is open at a time, and a sheet without its scrim is a bug waiting to
// happen. Close is always present and always last; `actions` is whatever that
// sheet adds before it. `label` is the accessible name, which can say more
// than the visible title has room for — the tabs sheet reads "Open tabs".
import * as React from "react";
import { X } from "lucide-react";

export default function PhoneSheet({
  label,
  title,
  actions,
  tall = false,
  onClose,
  children,
}: {
  label: string;
  title: string;
  actions?: React.ReactNode;
  /** Settings scrolls and earns the taller stop; a list of rows does not. */
  tall?: boolean;
  onClose: () => void;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <>
      <button
        type="button"
        className="phone-scrim"
        aria-label="Dismiss"
        onClick={onClose}
      />
      <div
        className={"phone-sheet" + (tall ? " phone-sheet-tall" : "")}
        role="dialog"
        aria-label={label}
      >
        <div className="phone-sheet-grab" aria-hidden />
        <div className="phone-sheet-head">
          <span className="phone-sheet-title">{title}</span>
          {actions}
          <button
            type="button"
            className="phone-sheet-action"
            aria-label="Close"
            onClick={onClose}
          >
            <X size={17} strokeWidth={1.8} />
          </button>
        </div>
        <div className="phone-sheet-body">{children}</div>
      </div>
    </>
  );
}
