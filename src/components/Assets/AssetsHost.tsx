import { lazy, Suspense } from "react";
import { create } from "zustand";

// Loaded the first time they open (three.js is large).
const AssetsDialog = lazy(() => import("./AssetsDialog").then((m) => ({ default: m.AssetsDialog })));
const GraphDialog = lazy(() => import("../Graph/GraphDialog").then((m) => ({ default: m.GraphDialog })));

type View = "assets" | "graph";
const useWorkspaceView = create<{ workspaceId: string | null; view: View }>(() => ({
  workspaceId: null,
  view: "assets",
}));

/** Opens the 3D assets browser of a workspace (sidebar button). */
export const openAssets = (workspaceId: string) => useWorkspaceView.setState({ workspaceId, view: "assets" });
/** Opens the notes graph of a workspace (sidebar button). */
export const openGraph = (workspaceId: string) => useWorkspaceView.setState({ workspaceId, view: "graph" });

export function AssetsHost() {
  const { workspaceId, view } = useWorkspaceView();
  if (!workspaceId) return null;
  const close = () => useWorkspaceView.setState({ workspaceId: null });
  const Dialog = view === "graph" ? GraphDialog : AssetsDialog;
  return (
    <Suspense fallback={null}>
      <Dialog workspaceId={workspaceId} onClose={close} />
    </Suspense>
  );
}
