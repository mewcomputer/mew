import { useLayoutEffect } from "react";
import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { ConnectionBanner } from "./connection-banner";
import { useSessionStore, type ConnectionState } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function ConnectionBannerPreview({
  connectionState,
  connectionError,
}: {
  connectionState: ConnectionState;
  connectionError: string | null;
}) {
  useLayoutEffect(() => {
    useSessionStore.setState({ connectionState, connectionError });
    return () => {
      useSessionStore.setState({ connectionState: "connected", connectionError: null });
    };
  }, [connectionState, connectionError]);

  return (
    <div className="relative min-h-24 p-4">
      <ConnectionBanner />
    </div>
  );
}

export default {
  title: "Layout/ConnectionBanner",
  parameters,
} satisfies StoryLiteMeta;

export const Reconnecting: StoryLiteStoryDefinition = {
  render: () => (
    <ConnectionBannerPreview
      connectionState="reconnecting"
      connectionError="WebSocket closed (1006)"
    />
  ),
};

export const DisconnectedWithError: StoryLiteStoryDefinition = {
  render: () => (
    <ConnectionBannerPreview
      connectionState="disconnected"
      connectionError="Failed to reach daemon at ws://127.0.0.1:9847/ws"
    />
  ),
};
