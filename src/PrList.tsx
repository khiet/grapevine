import { openUrl } from "@tauri-apps/plugin-opener";

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  created_at: string;
  section: "mine" | "participated" | "all";
}

export interface Snapshot {
  prs: PullRequest[];
  has_synced: boolean;
}

const SECTIONS = [
  { key: "mine", label: "Mine" },
  { key: "participated", label: "Participated" },
  { key: "all", label: "All" },
] as const;

// Compact single-unit age, Trailer-style: "now", "5m", "3h", "2d", "4w", "1y".
export function formatAge(iso: string, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - Date.parse(iso)) / 1000));
  const minutes = Math.floor(seconds / 60);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 52) return `${weeks}w`;
  return `${Math.floor(weeks / 52)}y`;
}

function PrRow({ pr }: { pr: PullRequest }) {
  return (
    <li>
      <button
        type="button"
        className="pr-row"
        onClick={() => openUrl(pr.url).catch(() => {})}
      >
        <span className="pr-title">{pr.title}</span>
        <span className="pr-meta">
          {pr.repo} #{pr.number} · {pr.author} · {formatAge(pr.created_at)}
        </span>
      </button>
    </li>
  );
}

function PrList({ prs }: { prs: PullRequest[] }) {
  return (
    <main className="pr-list">
      {SECTIONS.map(({ key, label }) => {
        const rows = prs.filter((pr) => pr.section === key);
        if (rows.length === 0) return null;
        return (
          <section key={key} className="pr-section">
            <h2 className="pr-section-label">{label}</h2>
            <ul>
              {rows.map((pr) => (
                <PrRow key={`${pr.repo}#${pr.number}`} pr={pr} />
              ))}
            </ul>
          </section>
        );
      })}
    </main>
  );
}

export default PrList;
