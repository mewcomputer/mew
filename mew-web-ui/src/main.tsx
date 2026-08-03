import React from "react";
import ReactDOM from "react-dom/client";
import {
  RouterProvider,
  createRouter,
} from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";
import { ThemeProvider } from "./lib/theme";
import { ErrorBoundary } from "./components/error-boundary";
import { initializeHost, resetHost } from "./lib/host";
import { Button } from "./components/ui/button";
import { RefreshCw, Server } from "lucide-react";
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

const root = ReactDOM.createRoot(document.getElementById("root")!);

function mountApp() {
  root.render(
    <React.StrictMode>
      <ThemeProvider>
        <AppShell />
      </ThemeProvider>
    </React.StrictMode>,
  );
}

function showHostError(error: unknown) {
  console.error("failed to initialize mew host", error);
  root.render(
    <HostStartupError
      error={error}
      onRetry={() => {
        resetHost();
        return initializeHost().then(mountApp).catch(showHostError);
      }}
    />,
  );
}

function HostStartupError({ error, onRetry }: { error: unknown; onRetry: () => Promise<void> }) {
  const [retrying, setRetrying] = React.useState(false);
  const message = error instanceof Error ? error.message : String(error);

  const handleRetry = () => {
    setRetrying(true);
    void onRetry().finally(() => setRetrying(false));
  };

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
      <section className="w-full max-w-md space-y-5 rounded-xl border border-border bg-card p-6 shadow-sm">
        <div className="flex items-start gap-3">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
            <Server className="h-4 w-4" />
          </span>
          <div className="space-y-1">
            <h1 className="text-base font-semibold">mew could not reach its daemon</h1>
            <p className="text-sm text-muted-foreground">
              The web client could not connect to the coding runtime.
            </p>
          </div>
        </div>
        <p className="rounded-md bg-muted px-3 py-2 font-mono text-xs text-muted-foreground">{message}</p>
        <Button onClick={handleRetry} disabled={retrying} className="w-full">
          <RefreshCw className={retrying ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
          {retrying ? "Retrying…" : "Retry connection"}
        </Button>
      </section>
    </main>
  );
}

void initializeHost().then(mountApp).catch(showHostError);
