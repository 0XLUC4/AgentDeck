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
