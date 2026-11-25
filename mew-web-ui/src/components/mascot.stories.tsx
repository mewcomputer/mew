import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { MASCOT_IDLES, MASCOT_STATUSES, MASCOT_STATUS_ALTERNATES, Mascot, type MascotState } from "./mascot";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function MascotGrid({ states }: { states: readonly MascotState[] }) {
  return (
    <div className="grid max-w-3xl grid-cols-2 gap-3 p-4 sm:grid-cols-4">
      {states.map((state) => (
        <div
          key={state}
          className="flex flex-col items-center gap-2 rounded-lg border border-border bg-card p-3 text-center"
        >
          <Mascot state={state} size={48} />
          <span className="font-mono text-[11px] text-muted-foreground">{state}</span>
        </div>
      ))}
    </div>
  );
}

export default {
  title: "Chat/Mascot",
  parameters,
} satisfies StoryLiteMeta;

export const HarnessStatuses: StoryLiteStoryDefinition = {
  render: () => <MascotGrid states={MASCOT_STATUSES} />,
};

export const StatusAlternates: StoryLiteStoryDefinition = {
  render: () => <MascotGrid states={MASCOT_STATUS_ALTERNATES} />,
};

export const PlayfulIdles: StoryLiteStoryDefinition = {
  render: () => <MascotGrid states={MASCOT_IDLES} />,
};

export const Neutral: StoryLiteStoryDefinition = {
  render: () => (
    <div className="flex items-center gap-3 p-6">
      <Mascot state="neutral" size={48} />
      <span className="font-mono text-xs text-muted-foreground">neutral</span>
    </div>
  ),
};
