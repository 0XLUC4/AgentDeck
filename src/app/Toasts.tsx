import { CircleAlert } from "lucide-react";
import { useDeck } from "../store/deck";

export function Toasts() {
  const toasts = useDeck((s) => s.toasts);
  return (
    <div role="status" aria-live="polite" className="fixed right-4 bottom-4 z-50 flex flex-col gap-2">
      {toasts.map((t) => (
        <p
          key={t.id}
          className="flex max-w-md items-start gap-2.5 rounded-lg border border-line-strong bg-raised px-3.5 py-2.5 text-[13px] text-fg"
        >
          <CircleAlert size={16} strokeWidth={1.5} className="mt-px shrink-0 text-wait" aria-hidden="true" />
          {t.message}
        </p>
      ))}
    </div>
  );
}
