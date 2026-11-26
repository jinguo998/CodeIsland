import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import i18n from "./lib/i18n";
import "./styles/theme.css";
import "./styles/app.css";

document.title = i18n.t("app.name", { defaultValue: "CodeIsland" });

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
