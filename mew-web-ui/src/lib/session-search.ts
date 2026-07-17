import type { ProjectInfo, SessionInfo } from "@mew/web-client";

const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

export function getSessionSearchText(
  session: SessionInfo,
  title?: string,
  summary?: string,
  content?: string,
): string {
  const timestamp = session.last_message_at ?? session.created_at;
  const date = new Date(toMillis(timestamp));
  return [
    title,
    summary,
    session.summary,
    session.first_message,
    content,
    session.cwd,
    session.model,
    date.toISOString().slice(0, 10),
    date.toLocaleDateString(),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

export function filterSessions(
  sessions: SessionInfo[],
  query: string,
  titles?: ReadonlyMap<string, string>,
  summaries?: ReadonlyMap<string, string>,
  content?: ReadonlyMap<string, string>,
): SessionInfo[] {
  const filters = parseQuery(query);
  if (
    filters.terms.length === 0 &&
    !filters.project &&
    !filters.folder &&
    !filters.before &&
    !filters.after
  )
    return sessions;

  return sessions.filter((session) => {
    const text = getSessionSearchText(
      session,
      titles?.get(session.session_id),
      summaries?.get(session.session_id),
      content?.get(session.session_id),
    );
    const folder = (session.cwd ?? "").toLowerCase();
    const timestamp = session.last_message_at ?? session.created_at;
    const day = new Date(toMillis(timestamp));
    const date = new Date(day.getFullYear(), day.getMonth(), day.getDate()).getTime();

    return (
      filters.terms.every((term) => text.includes(term)) &&
      (!filters.project || folder.includes(filters.project)) &&
      (!filters.folder || folder.includes(filters.folder)) &&
      (!filters.after || date >= filters.after) &&
      (!filters.before || date <= filters.before)
    );
  });
}

export function filterProjects(projects: ProjectInfo[], query: string): ProjectInfo[] {
  const filters = parseQuery(query);
  if (filters.terms.length === 0 && !filters.project && !filters.folder) return projects;

  return projects.filter((project) => {
    const text = `${project.display_name} ${project.path}`.toLowerCase();
    return (
      filters.terms.every((term) => text.includes(term)) &&
      (!filters.project || text.includes(filters.project)) &&
      (!filters.folder || text.includes(filters.folder))
    );
  });
}

export function formatSessionDate(timestamp?: number): string {
  if (!timestamp) return "No activity yet";
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" }).format(
    new Date(toMillis(timestamp)),
  );
}

/** The wire docs say seconds, while older daemon builds sent milliseconds. */
function toMillis(timestamp: number): number {
  return timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp;
}

export function projectName(cwd?: string): string {
  if (!cwd) return "Unknown project";
  return cwd.split(/[\\/]/).filter(Boolean).pop() ?? cwd;
}

function parseQuery(query: string) {
  const filters: {
    terms: string[];
    project?: string;
    folder?: string;
    before?: number;
    after?: number;
  } = { terms: [] };

  for (const raw of query.trim().toLowerCase().split(/\s+/)) {
    if (!raw) continue;
    const separator = raw.indexOf(":");
    if (separator > 0) {
      const key = raw.slice(0, separator);
      const value = raw.slice(separator + 1);
      if ((key === "project" || key === "folder") && value) {
        filters[key] = value;
        continue;
      }
      if ((key === "before" || key === "after") && DATE_PATTERN.test(value)) {
        const time = parseLocalDate(value);
        if (time !== undefined) filters[key] = time;
        continue;
      }
    }
    filters.terms.push(raw);
  }

  return filters;
}

function parseLocalDate(value: string): number | undefined {
  const [year, month, day] = value.split("-").map(Number);
  const time = new Date(year!, month! - 1, day!).getTime();
  return Number.isNaN(time) ? undefined : time;
}
