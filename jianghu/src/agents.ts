import type { SpriteRole } from './agent-sprites.ts';
import { districtCast } from './district.ts';

export const statusLabels = {
  idle: 'Idle',
  running: 'Running',
  waiting: 'Waiting',
  needs_approval: 'Needs approval',
  done: 'Done',
  failed: 'Failed',
};

export type AgentStatus = keyof typeof statusLabels;

interface Approval {
  id: string;
  at: number;
  summary: string;
}

export interface Agent {
  id: string;
  role: SpriteRole;
  position: readonly [number, number, number];
  name: string;
  status: AgentStatus;
  task: string;
  activity: string;
  approval?: Approval;
}

export type AgentEvent =
  | { type: 'agent.status'; id: string; status: AgentStatus; activity: string }
  | { type: 'approval.requested'; id: string; approvalId: string; at: number; summary: string }
  | { type: 'approval.resolved'; id: string; approvalId: string; outcome: 'approved' | 'declined' };

const agentDetails: Omit<Agent, 'role' | 'position'>[] = [
  {
    id: 'lin',
    name: 'Lin',
    status: 'running',
    task: 'Survey the upper canal',
    activity: 'Comparing the notes from two inspection routes.',
  },
  {
    id: 'mei',
    name: 'Mei',
    status: 'needs_approval',
    task: 'Check the sluice controls',
    activity: 'The patch passed its six checks.',
    approval: {
      id: 'sluice-review',
      at: 10,
      summary: 'Close the Waterworks sluice to check how the canal backs up.',
    },
  },
  {
    id: 'shen',
    name: 'Shen',
    status: 'waiting',
    task: 'Inspect the lower terraces',
    activity: 'Waiting for the next inspection record.',
  },
  {
    id: 'yu',
    name: 'Yu',
    status: 'idle',
    task: 'Draft the maintenance notes',
    activity: 'Ready for the next task.',
  },
];
export const demoAgents = agentDetails.map((agent): Agent => {
  const character = districtCast.find(({ id }) => id === agent.id);
  if (!character) throw new Error(`No district character for ${agent.id}`);
  return { ...character, ...agent };
});

export function createAgentStore(initial: readonly Agent[]) {
  const agents = new Map(initial.map((agent) => [agent.id, { ...agent }]));
  const listeners = new Set<(event: AgentEvent) => void>();
  return {
    get: (id: string) => agents.get(id),
    all: () => [...agents.values()],
    pending: () =>
      [...agents.values()]
        .filter((agent): agent is Agent & { approval: Approval } => Boolean(agent.approval))
        .sort((a, b) => a.approval.at - b.approval.at || a.id.localeCompare(b.id)),
    subscribe(listener: (event: AgentEvent) => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    dispatch(event: AgentEvent) {
      const agent = agents.get(event.id);
      if (!agent) return false;
      let update: Partial<Agent>;
      switch (event.type) {
        case 'agent.status':
          if (
            agent.approval ||
            !Object.hasOwn(statusLabels, event.status) ||
            event.status === 'needs_approval'
          )
            return false;
          update = { status: event.status, activity: event.activity };
          break;
        case 'approval.requested':
          if (agent.approval) return false;
          update = {
            status: 'needs_approval',
            approval: { id: event.approvalId, at: event.at, summary: event.summary },
          };
          break;
        case 'approval.resolved': {
          if (
            !agent.approval ||
            agent.approval.id !== event.approvalId ||
            !['approved', 'declined'].includes(event.outcome)
          )
            return false;
          const approved = event.outcome === 'approved';
          update = {
            approval: undefined,
            status: approved ? 'running' : 'waiting',
            activity: approved
              ? 'Approval recorded in the demo. Work resumed.'
              : 'Request declined. Waiting for direction.',
          };
          break;
        }
        default:
          return false;
      }
      agents.set(agent.id, { ...agent, ...update });
      for (const listener of listeners) listener(event);
      return true;
    },
  };
}
