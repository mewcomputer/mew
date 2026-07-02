/**
 * Module-level navigation function, populated by the root route component.
 * Allows non-React code (e.g. the store bridge, notification click handlers)
 * to navigate without being inside the React tree.
 *
 * Mirrors the pattern used by `client-ref.ts`.
 */
export const routerRef: { navigate: ((sessionId: string) => void) | null } = {
  navigate: null,
};

/** Navigate to a session. Also persists the session id to localStorage. */
export function navigateToSession(sessionId: string) {
  localStorage.setItem("mew.sessionId", sessionId);
  if (routerRef.navigate) {
    routerRef.navigate(sessionId);
  }
}
