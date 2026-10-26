import { useDeck } from "../store/deck";

export function Toasts() {
  const toasts = useDeck((s) => s.toasts);
  return (
    <div role="status" aria-live="polite" className="fixed right-4 bottom-4 z-50 flex flex-col gap-2">
      {toasts.map((t) => (
        <p key={t.id} className="max-w-md rounded-lg border border-red-800 bg-zinc-900 px-4 py-2 text-sm shadow-lg">
          {t.message}
        </p>
      ))}
    </div>
  );
}
