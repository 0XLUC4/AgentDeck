// Entry point of a detached pane window (see pane.html, opened by components/Detached/host.ts).
import React from "react";
import ReactDOM from "react-dom/client";
import { DetachedPane } from "./components/Detached/DetachedPane";
import "./index.css";

const paneId = new URLSearchParams(window.location.search).get("pane");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {paneId ? <DetachedPane paneId={paneId} /> : <p className="p-4 text-muted">Pane introuvable.</p>}
  </React.StrictMode>,
);
