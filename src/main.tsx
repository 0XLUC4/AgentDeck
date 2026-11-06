import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { startAppearance } from "./lib/theme";
import "./index.css";

startAppearance();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// The splash in index.html covers startup; fade it out once the first frame of the app is painted.
requestAnimationFrame(() => {
  const splash = document.getElementById("splash");
  splash?.classList.add("gone");
  setTimeout(() => splash?.remove(), 300);
});
