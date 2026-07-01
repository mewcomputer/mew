import React from "react";
import ReactDOM from "react-dom/client";
import {
  RouterProvider,
  createRouter,
} from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";
import { ThemeProvider } from "./lib/theme";
import { ErrorBoundary } from "./components/error-boundary";
import "./index.css";

const appRouter = createRouter({
  routeTree,
  defaultPreload: "intent",
});

function AppShell() {
  return (
    <ErrorBoundary title="App crashed">
      <RouterProvider router={appRouter} />
    </ErrorBoundary>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider>
      <AppShell />
    </ThemeProvider>
  </React.StrictMode>,
);
