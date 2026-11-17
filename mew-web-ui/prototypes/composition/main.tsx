import { createRoot } from "react-dom/client";
import { CompositionApp, queryHost, type CompositionBootstrap } from "./app";
import { BrowserSurface } from "./surface";
import "../../src/index.css";
import "./style.css";

const root = createRoot(document.getElementById("root")!);
void queryHost<CompositionBootstrap>({ type: "bootstrap" }).then((bootstrap) => {
  root.render(<CompositionApp bootstrap={bootstrap} surface={<BrowserSurface bootstrap={bootstrap} />} />);
}).catch((error: unknown) => { root.render(<p role="alert">{String(error)}</p>); });
