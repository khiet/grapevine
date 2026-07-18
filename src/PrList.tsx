import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  avatar_url: string;
  owner_avatar_url: string;
  created_at: string;
  updated_at: string;
  section: "mine" | "participated" | "all";
  /** Head commit's CI rollup. Only "failing" is rendered; the rest, including
   * "none" (no checks configured), leave the row undecorated. */
  ci_status: "passing" | "failing" | "pending" | "none";
  unread_count: number;
}

export interface MergedPr {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  avatar_url: string;
  owner_avatar_url: string;
  merged_at: string;
}

export interface Snapshot {
  prs: PullRequest[];
  merged: MergedPr[];
  has_synced: boolean;
  /** Epoch ms of the last successful sync; null before the first one. */
  last_sync_at: number | null;
  /** User-facing message of the most recent failure; cleared by a success. */
  sync_error: string | null;
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

// Footer label for the last successful sync, reusing the row-timestamp
// format: "Synced 14:59" today, then "Synced Yesterday", "Synced 14 Jul".
export function formatLastSync(
  ms: number | null,
  now: Date = new Date(),
): string | null {
  if (ms === null) return null;
  return `Synced ${formatUpdated(new Date(ms).toISOString(), now)}`;
}

export interface RepoGroup {
  repo: string;
  prs: PullRequest[];
}

// Groups ordered by their most recently updated PR, so the repo that just moved
// leads; within a group the caller's order is preserved. An unparseable
// updated_at sorts as the epoch rather than throwing.
export function groupByRepo(prs: PullRequest[]): RepoGroup[] {
  const groups = new Map<string, PullRequest[]>();
  for (const pr of prs) {
    const existing = groups.get(pr.repo);
    if (existing) existing.push(pr);
    else groups.set(pr.repo, [pr]);
  }
  const latest = (rows: PullRequest[]) =>
    rows.reduce((max, pr) => Math.max(max, Date.parse(pr.updated_at) || 0), 0);
  return [...groups.entries()]
    .map(([repo, rows]) => ({ repo, prs: rows }))
    .sort((a, b) => latest(b.prs) - latest(a.prs));
}

// The round author avatar with a small square organization badge (the repo
// owner's avatar) on its corner: the face says who, the badge says which org.
// The badge has no placeholder by design — a missing or broken owner avatar
// simply shows no badge, keeping the corner clean rather than grey.
function PrAvatar({
  avatarUrl,
  ownerAvatarUrl,
}: {
  avatarUrl: string;
  ownerAvatarUrl: string;
}) {
  return (
    <span className="pr-avatar-wrap">
      <img
        className="pr-avatar"
        alt=""
        src={avatarUrl || AVATAR_PLACEHOLDER}
        onError={(e) => {
          e.currentTarget.src = AVATAR_PLACEHOLDER;
        }}
      />
      {ownerAvatarUrl && (
        <img
          className="pr-org-badge"
          alt=""
          src={ownerAvatarUrl}
          onError={(e) => {
            e.currentTarget.style.display = "none";
          }}
        />
      )}
    </span>
  );
}

// showRepo is false inside a repo group, where the header already names it.
function PrRow({ pr, showRepo = true }: { pr: PullRequest; showRepo?: boolean }) {
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
        <PrAvatar avatarUrl={pr.avatar_url} ownerAvatarUrl={pr.owner_avatar_url} />
        <span className="pr-text">
          <span className="pr-title-row">
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">{formatUpdated(pr.updated_at)}</span>
          </span>
          <span className="pr-origin">
            <span className="pr-repo">
              {showRepo ? `${pr.repo} #${pr.number}` : `#${pr.number}`}
            </span>
            <span className="pr-author">@{pr.author}</span>
            {/* A single mark for a broken build. Passing and pending stay
                undecorated so a quiet row keeps meaning "nothing needs you".
                Distinct from the red unread badge in hue and meaning: orange
                here says "your build is broken", red there says "someone
                spoke". PAVE-3615 reuses this same dot for "PR is blocked". */}
            {pr.ci_status === "failing" && (
              <span
                className="pr-ci"
                title="Checks failing"
                aria-label="Checks failing"
              />
            )}
          </span>
        </span>
      </button>
    </li>
  );
}

/* Two sibling buttons, not a dismiss nested inside the row button, which
   would be invalid HTML. No mark_read here: merged PRs carry no unread
   state. */
function MergedRow({ pr }: { pr: MergedPr }) {
  return (
    <li className="pr-row-split">
      <button
        type="button"
        className="pr-row"
        onClick={() => openUrl(pr.url).catch(() => {})}
      >
        {/* The gutter span stays even without a badge so avatars align. */}
        <span />
        <PrAvatar avatarUrl={pr.avatar_url} ownerAvatarUrl={pr.owner_avatar_url} />
        <span className="pr-text">
          <span className="pr-title-row">
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">{formatUpdated(pr.merged_at)}</span>
          </span>
          <span className="pr-origin">
            <span className="pr-repo">
              {pr.repo} #{pr.number}
            </span>
            <span className="pr-author">@{pr.author}</span>
          </span>
        </span>
      </button>
      <button
        type="button"
        className="pr-dismiss"
        aria-label="Dismiss"
        onClick={() =>
          invoke("dismiss_merged", { key: `${pr.repo}#${pr.number}` }).catch(
            () => {},
          )
        }
      >
        ×
      </button>
    </li>
  );
}

function PrList({ prs, merged }: { prs: PullRequest[]; merged: MergedPr[] }) {
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
            {key === "all" ? (
              groupByRepo(rows).map(({ repo, prs: group }) => (
                <div key={repo} className="pr-repo-group">
                  <div className="pr-repo-header">
                    <h3 className="pr-repo-name">{repo}</h3>
                    <span className="pr-repo-count">{group.length}</span>
                  </div>
                  <ul>
                    {group.map((pr) => (
                      <PrRow
                        key={`${pr.repo}#${pr.number}`}
                        pr={pr}
                        showRepo={false}
                      />
                    ))}
                  </ul>
                </div>
              ))
            ) : (
              <ul>
                {rows.map((pr) => (
                  <PrRow key={`${pr.repo}#${pr.number}`} pr={pr} />
                ))}
              </ul>
            )}
          </section>
        );
      })}
      {merged.length > 0 && (
        <section className="pr-section">
          <div className="pr-section-header">
            <h2 className="pr-section-label">Merged</h2>
            <span className="pr-section-count">{merged.length}</span>
            <button
              type="button"
              className="pr-section-clear"
              onClick={() => invoke("clear_merged").catch(() => {})}
            >
              Clear all
            </button>
          </div>
          <ul>
            {merged.map((pr) => (
              <MergedRow key={`${pr.repo}#${pr.number}`} pr={pr} />
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

export default PrList;
