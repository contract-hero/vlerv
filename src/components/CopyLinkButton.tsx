// The "Copy link" button, shared by the Beam send dialog and the Remote
// pairing invite in Settings. Both hand the user a one-shot link and both
// want the same brief "Copied" acknowledgement, so they share one button.
import * as React from "react";
import { Check, Copy } from "lucide-react";
import { useCopyFeedback } from "../hooks/useCopyFeedback";

export default function CopyLinkButton({
  link,
  testId,
}: {
  link: string;
  /** Only the Beam instance is addressed by a test today; omitted renders no
   *  attribute at all. */
  testId?: string;
}): React.ReactElement {
  const { copied, copy } = useCopyFeedback();
  return (
    <button
      type="button"
      className="button"
      data-testid={testId}
      onClick={() => copy(link)}
    >
      {copied ? <Check size={13} strokeWidth={2.5} /> : <Copy size={13} strokeWidth={2} />}
      {copied ? "Copied" : "Copy link"}
    </button>
  );
}
