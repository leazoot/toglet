import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { installWebviewGuards } from "./shell/guards";
import "./styles/global.css";

installWebviewGuards(document);

const rootElement = document.getElementById("root");
if (rootElement === null) {
  // index.html always ships this node; its absence means the bundled shell is corrupt,
  // and there is no UI left to report the failure through.
  throw new Error("Root element #root is missing from the document");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
