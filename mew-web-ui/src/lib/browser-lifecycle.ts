export interface BrowserEventIdentity {
  tabId?: string;
  url?: string;
}

export interface BrowserTabIdentity {
  tabId: string;
  url: string;
  active?: boolean;
}

/**
 * Accept tagged events from the active tab. Untagged events are supported for
 * older daemons, but only when their URL still matches the active tab.
 */
export function acceptsBrowserEvent(
  event: BrowserEventIdentity,
  activeTab: BrowserTabIdentity,
): boolean {
  if (event.tabId) return event.tabId === activeTab.tabId;
  if (activeTab.active === false) return false;
  return Boolean(activeTab.url) && event.url === activeTab.url;
}
