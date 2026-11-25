import { createContext, useContext } from "react";

/** Measured height of the composer region floating over the transcript. The
 *  transcript clears it and re-pins when it grows, so the newest lines are never
 *  left underneath it. */
export const ComposerHeightContext = createContext(0);

export function useComposerHeight(): number {
  return useContext(ComposerHeightContext);
}
