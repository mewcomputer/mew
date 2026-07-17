import type { AlertKind, SessionInfo } from "@mew/web-client";

export type AttentionKind = "permission" | "question" | "failed";

export interface SessionAttention {
  kind: AttentionKind;
  label: string;
  rank: number;
  count: number;
}

const ALERT_TO_ATTENTION: Partial<Record<AlertKind, AttentionKind>> = {
  permission_needed: "permission",
  input_needed: "question",
  turn_failed: "failed",
};

const ATTENTION_META: Record<AttentionKind, Omit<SessionAttention, "count">> = {
  permission: { kind: "permission", label: "Permissions needed", rank: 0 },
  question: { kind: "question", label: "Question · needs input", rank: 1 },
  failed: { kind: "failed", label: "Turn failed", rank: 2 },
};

/** Return all actionable attention states, ordered by what blocks progress most. */
export function getSessionAttention(
  session: SessionInfo,
  alertKinds: readonly AlertKind[] = [],
): SessionAttention[] {
  const counts: Record<AttentionKind, number> = {
    permission: session.pending_permissions ?? 0,
    question: session.pending_questions ?? 0,
    failed: 0,
  };

  for (const alertKind of alertKinds) {
    const kind = ALERT_TO_ATTENTION[alertKind];
    if (kind === "failed") counts.failed = Math.max(counts.failed, 1);
  }

  return (Object.keys(ATTENTION_META) as AttentionKind[])
    .filter((kind) => counts[kind] > 0)
    .sort((a, b) => ATTENTION_META[a].rank - ATTENTION_META[b].rank)
    .map((kind) => ({ ...ATTENTION_META[kind], count: counts[kind] }));
}

/** Sort sessions without allowing a quiet recent session to outrank attention. */
export function compareSessionsByAttention(
  a: SessionInfo,
  b: SessionInfo,
  alertsBySession: ReadonlyMap<string, readonly AlertKind[]> = new Map(),
): number {
  const aAttention = getSessionAttention(a, alertsBySession.get(a.session_id));
  const bAttention = getSessionAttention(b, alertsBySession.get(b.session_id));
  const aRank = aAttention[0]?.rank ?? 3;
  const bRank = bAttention[0]?.rank ?? 3;
  if (aRank !== bRank) return aRank - bRank;

  if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
  return (b.last_message_at ?? b.created_at) - (a.last_message_at ?? a.created_at);
}

export function attentionKindLabel(kind: AttentionKind): string {
  return ATTENTION_META[kind].label;
}
