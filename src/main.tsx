import React from "react";
import ReactDOM from "react-dom/client";
// Design tokens (light/dark via [data-theme]) — must load before app styles.
import "./styles/tokens.css";
// Wire Monaco's local worker + themes before any editor is created.
import "./monaco/setup";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
