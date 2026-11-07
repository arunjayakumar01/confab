import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "@fontsource-variable/figtree";
import "@fontsource-variable/bricolage-grotesque";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Dev hooks for poking the store and injecting relay frames from the console; stripped in production builds.
if (import.meta.env.DEV) {
  void import("./store").then(({ useStore }) => ((window as unknown as { __prism: unknown }).__prism = useStore));
  void import("./actions").then(({ injectFrame }) => ((window as unknown as { __prismFrame: unknown }).__prismFrame = injectFrame));
}
