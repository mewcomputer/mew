import { useLayoutEffect } from "react";
import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { PermissionToast } from "./permission-toast";
import { useSessionStore, type PendingPermission } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function PermissionToastPreview({ permission }: { permission: PendingPermission }) {
  useLayoutEffect(() => {
    useSessionStore.setState({ pendingPermissions: [permission] });
    return () => {
      useSessionStore.setState({ pendingPermissions: [] });
    };
  }, [permission]);

  return (
    <div className="relative min-h-56 p-4">
      <PermissionToast onResolve={() => useSessionStore.setState({ pendingPermissions: [] })} />
    </div>
  );
}

export default {
  title: "Chat/PermissionToast",
  parameters,
} satisfies StoryLiteMeta;

export const BashCommand: StoryLiteStoryDefinition = {
  render: () => (
    <PermissionToastPreview
      permission={{
        requestId: "perm-1",
        toolName: "bash",
        input: {
          command: "cargo test -p mew-tools --locked",
          description: "Run mew-tools unit tests",
        },
      }}
    />
  ),
};
