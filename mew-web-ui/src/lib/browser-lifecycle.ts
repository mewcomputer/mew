export interface BrowserEventIdentity {
  tabId?: string;
  url?: string;
}

export interface BrowserTabIdentity {
  tabId: string;
  url: string;
  active?: boolean;
}

export interface NativeBrowserEventIdentity {
  kind: "address_changed" | "title_changed";
  owner?: string;
  title?: string;
  url?: string;
}

export interface NativeBrowserTabIdentity extends BrowserTabIdentity {
  visible: boolean;
  loading: boolean;
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

/**
 * Native CEF is a singleton surface. Only the visible owner may consume its
 * events, and a loading tab must not accept a callback for an older URL.
 */
export function acceptsNativeBrowserEvent(
  event: NativeBrowserEventIdentity,
  activeTab: NativeBrowserTabIdentity,
): boolean {
  if (!activeTab.active || !activeTab.visible || event.owner !== activeTab.tabId) {
    return false;
  }
  if (activeTab.loading && activeTab.url && event.url && event.url !== activeTab.url) {
    return false;
  }
  if (event.kind === "title_changed" && activeTab.url && event.url && event.url !== activeTab.url) {
    return false;
  }
  return true;
}
