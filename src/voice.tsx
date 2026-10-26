// Entry point of the floating voice pill window (see voice.html).
import React from "react";
import ReactDOM from "react-dom/client";
import { VoicePill } from "./components/VoicePill/VoicePill";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <VoicePill />
  </React.StrictMode>,
);
