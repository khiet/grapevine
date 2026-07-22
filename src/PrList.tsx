import type { ReactNode } from "react";
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
  /** Why the PR is blocked, already in severity order; empty means no pill.
   * Composed at the Rust boundary — the row only maps keys to labels. */
  blocked_reasons: BlockedReason[];
  /** Drafts render a neutral pill; the backend suppresses their dot. */
  is_draft: boolean;
  /** The viewer's review is requested and not yet acted on; renders the
   * glasses glyph with an incoming arrow. Backend-computed, suppressed on
   * drafts, and self-clearing once the viewer reviews. */
  review_requested: boolean;
  /** One of the viewer's own PRs is waiting on a reviewer; renders the glasses
   * glyph with an outgoing arrow. The Mine-only mirror of review_requested
   * (the two never share a row), suppressed on drafts, and self-clearing as
   * reviewers submit. */
  awaiting_review: boolean;
  /** Files touched, straight from GitHub. GitHub computes this lazily, so a
   * freshly opened PR can report 0; the row hides the count when it is 0 (see
   * {@link changedFilesLabel}). */
  changed_files: number;
  unread_count: number;
}

export type BlockedReason = "conflict" | "ci" | "review" | "behind";

/* Neutral PR-property wording, deliberately not "you must act": the section
   the row sits in supplies the "is this mine to fix?" context. */
const BLOCKED_LABELS: Record<BlockedReason, string> = {
  conflict: "Merge conflict",
  ci: "CI failing",
  review: "Changes requested",
  behind: "Behind base",
};

// Joined reason labels in the backend's fixed order; feeds the +N pill's
// tooltip (called with the reasons the primary pill did not show).
export function blockedTitle(reasons: BlockedReason[]): string {
  return reasons.map((reason) => BLOCKED_LABELS[reason]).join("; ");
}

// The visible pill texts for a blocked row: the first reason spelled out,
// then "+N" for the rest, so a doubly-blocked row stays width-bounded
// ("Merge conflict" "+1") instead of listing everything inline. The first
// reason wins because the backend orders by severity (conflict, ci, review,
// behind).
export function blockedPills(reasons: BlockedReason[]): string[] {
  if (reasons.length === 0) return [];
  const pills = [BLOCKED_LABELS[reasons[0]]];
  if (reasons.length > 1) pills.push(`+${reasons.length - 1}`);
  return pills;
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

// The row's size signal: how many files the PR touches. Returns null when the
// count is 0, which the row treats as "render nothing": 0 means either a
// genuinely empty PR or GitHub not having computed the count yet, and "0 files"
// reads as noise in both cases. Singular label so a one-file PR does not read
// "1 files".
export function changedFilesLabel(pr: PullRequest): string | null {
  if (pr.changed_files === 0) {
    return null;
  }
  return pr.changed_files === 1 ? "1 file" : `${pr.changed_files} files`;
}

// The fields a filter term can match: title, repo, "#number", and author.
// Only PullRequest and MergedPr flow through here, and both carry these.
type Filterable = Pick<PullRequest, "title" | "repo" | "author" | "number">;

// True when every whitespace-separated term appears somewhere in the row's
// searchable text (case-insensitive). Terms are AND'd, so "acme fix" narrows
// rather than widens. An empty or whitespace-only query matches everything.
export function matchesFilter(item: Filterable, query: string): boolean {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;
  const haystack =
    `${item.title} ${item.repo} #${item.number} ${item.author}`.toLowerCase();
  return terms.every((term) => haystack.includes(term));
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

// A grey glyph in the row's right-edge marker cluster: a 13px stroke icon
// whose meaning lives in the hover tooltip and aria-label, both fed by `tip`.
// `wide` lets it hold a direction arrow beside the glasses. The draft pill and
// the blocked dot are not these: they carry their own styling, not a glyph.
function RowMark({
  tip,
  wide,
  children,
}: {
  tip: string;
  wide?: boolean;
  children: ReactNode;
}) {
  return (
    <span
      className={wide ? "pr-glyph pr-glyph-wide pr-tip" : "pr-glyph pr-tip"}
      role="img"
      data-tip={tip}
      aria-label={tip}
    >
      {children}
    </span>
  );
}

// The review glasses, shared by both review-request directions.
function Glasses() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="13"
      height="13"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="6" cy="15" r="4" />
      <circle cx="18" cy="15" r="4" />
      <path d="M14 15a2 2 0 0 0-4 0" />
      <path d="M2.5 13 5 7c.7-1.3 1.4-2 3-2" />
      <path d="M21.5 13 19 7c-.7-1.3-1.5-2-3-2" />
    </svg>
  );
}

// The direction arrow that pairs with the glasses. "in" points at the viewer
// (someone requested their review); "out" points away (the viewer is waiting
// on a reviewer). A hair heavier stroke than the glasses so it stays legible
// at its narrow width.
function DirectionArrow({ dir }: { dir: "in" | "out" }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="9"
      height="13"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {dir === "in" ? (
        <>
          <path d="M20 12H6" />
          <path d="M11 6l-5 6 5 6" />
        </>
      ) : (
        <>
          <path d="M4 12h14" />
          <path d="M13 6l5 6-5 6" />
        </>
      )}
    </svg>
  );
}

// showRepo is false inside a repo group, where the header already names it.
function PrRow({ pr, showRepo = true }: { pr: PullRequest; showRepo?: boolean }) {
  const unread = pr.unread_count > 0;
  const files = changedFilesLabel(pr);
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
            {/* The PR's size: files touched. Fixed width (flex: none), so the
                repo name truncates first under a tight row. Hidden when the PR
                has no computed changes (see changedFilesLabel), and yields on
                a blocked row: the reason pill matters more than the size, and
                the count returns the moment the block clears. */}
            {files && pr.blocked_reasons.length === 0 && (
              <span className="pr-diffstat">{files}</span>
            )}
            {/* A neutral state pill, never a signal. The backend suppresses
                the other markers on a draft (the author has not declared
                readiness), so a draft row shows only this pill, with room to
                spare. */}
            {pr.is_draft && <span className="pr-draft">Draft</span>}
            {/* The action markers cluster at the row's right edge as one group
                instead of scattering through the metadata: the review glyph
                (incoming or outgoing, never both on one row), then the blocked
                pills. Never shown on a draft (the backend suppresses all
                three), so this and the draft pill are exclusive. */}
            {(pr.review_requested ||
              pr.awaiting_review ||
              pr.blocked_reasons.length > 0) && (
              <span className="pr-marks">
                {/* Glasses with an incoming arrow: your review is requested. A
                    grey mark like the draft pill, not a status dot: an
                    invitation to act, outside the orange dot's "something is
                    stuck" vocabulary. Self-clears once you review. */}
                {pr.review_requested && (
                  <RowMark tip="Review requested" wide>
                    <DirectionArrow dir="in" />
                    <Glasses />
                  </RowMark>
                )}
                {/* Glasses with an outgoing arrow: one of your PRs is waiting on
                    a reviewer. The Mine-only mirror of the incoming glyph, so
                    it never shares a row with it. Self-clears as reviewers
                    submit (the backend drops the request). */}
                {pr.awaiting_review && (
                  <RowMark tip="Awaiting review" wide>
                    <Glasses />
                    <DirectionArrow dir="out" />
                  </RowMark>
                )}
                {/* The blocked reason, spelled out in a pill (the Draft
                    pill's shape in the blocked orange) so "why is this stuck"
                    reads without hovering. Extra reasons collapse into a +N
                    pill whose tooltip names them; the primary pill needs no
                    tooltip because it IS its own label. In-flight and healthy
                    states stay undecorated so a quiet row keeps meaning
                    "nothing is stuck". Distinct from the red unread badge in
                    hue and meaning: orange here says "this PR is blocked",
                    red there says "someone spoke". */}
                {pr.blocked_reasons.length > 0 && (
                  <span className="pr-blocked-pill">
                    {blockedPills(pr.blocked_reasons)[0]}
                  </span>
                )}
                {pr.blocked_reasons.length > 1 && (
                  <span
                    className="pr-blocked-pill pr-tip"
                    role="img"
                    data-tip={blockedTitle(pr.blocked_reasons.slice(1))}
                    aria-label={blockedTitle(pr.blocked_reasons.slice(1))}
                  >
                    {blockedPills(pr.blocked_reasons)[1]}
                  </span>
                )}
              </span>
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
