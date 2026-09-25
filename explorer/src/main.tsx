import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Api } from "./api";
import { App } from "./app";
import { Router } from "./router";
import "./styles.css";

const root = document.getElementById("app");
if (!root) throw new Error("index.html has no #app element");

createRoot(root).render(
  <StrictMode>
    <App api={new Api()} router={new Router()} />
  </StrictMode>,
);
