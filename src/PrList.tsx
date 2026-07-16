import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  avatar_url: string;
  created_at: string;
  updated_at: string;
  section: "mine" | "participated" | "all";
  unread_count: number;
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

// Neutral grey silhouette shown while an avatar loads and when it fails.
const AVATAR_PLACEHOLDER =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 40 40'%3E%3Ccircle cx='20' cy='20' r='20' fill='%23b8b8bd'/%3E%3Ccircle cx='20' cy='15' r='6.5' fill='%236e6e73'/%3E%3Cpath d='M6 40a14 14 0 0 1 28 0z' fill='%236e6e73'/%3E%3C/svg%3E";

// Fixed English month names and 24-hour time, not locale formatting: the
// timestamp column is a fixed 52px, sized for exactly these forms.
const MONTHS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// Short updated timestamp in local time: "14:59" for today, "Yesterday",
// "22 Jan" for anything older. The split is by calendar day, not 24 hours.
export function formatUpdated(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  const startOfDay = (d: Date) =>
    new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  // Rounding absorbs DST days, which are not exactly 24 hours long.
  const days = Math.round((startOfDay(now) - startOfDay(then)) / 86_400_000);
  if (days <= 0) {
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${pad(then.getHours())}:${pad(then.getMinutes())}`;
  }
  return days === 1 ? "Yesterday" : `${then.getDate()} ${MONTHS[then.getMonth()]}`;
}

export function totalUnread(prs: PullRequest[]): number {
  return prs.reduce((sum, pr) => sum + pr.unread_count, 0);
}

function PrRow({ pr }: { pr: PullRequest }) {
  const unread = pr.unread_count > 0;
  const open = () => {
    openUrl(pr.url).catch(() => {});
    // The backend clears the badge and pushes a fresh snapshot.
    invoke("mark_read", { key: `${pr.repo}#${pr.number}` }).catch(() => {});
  };
  return (
    <li>
      <button
        type="button"
        className={unread ? "pr-row is-unread" : "pr-row"}
        onClick={open}
      >
        {/* The gutter span stays even without a badge so avatars align. */}
        {unread ? <span className="pr-unread">{pr.unread_count}</span> : <span />}
        <img
          className="pr-avatar"
          alt=""
          src={pr.avatar_url || AVATAR_PLACEHOLDER}
          onError={(e) => {
            e.currentTarget.src = AVATAR_PLACEHOLDER;
          }}
        />
        <span className="pr-text">
          <span className="pr-title-row">
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">{formatUpdated(pr.updated_at)}</span>
          </span>
          <span className="pr-origin">
            <span className="pr-repo">
              {pr.repo} #{pr.number}
            </span>
            <span className="pr-author">@{pr.author}</span>
          </span>
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
            <div className="pr-section-header">
              <h2 className="pr-section-label">{label}</h2>
              <span className="pr-section-count">{rows.length}</span>
            </div>
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
