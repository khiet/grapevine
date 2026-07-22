import { useState, type ReactNode } from "react";
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
  /** Drafts render a neutral pill; the backend suppresses their blocked
   * reasons and review markers. */
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
  unread_count: number;
}

export type BlockedReason = "conflict" | "ci" | "review" | "threads" | "behind";

/* Neutral PR-property wording, deliberately not "you must act": the section
   the row sits in supplies the "is this mine to fix?" context. */
const BLOCKED_LABELS: Record<BlockedReason, string> = {
  conflict: "Merge conflict",
  ci: "CI failing",
  review: "Changes requested",
  threads: "Unresolved threads",
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
// threads, behind).
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

/** Every collapsible top-level section; repo sub-groups inside "All" are
 * deliberately not collapsible. */
export type SectionKey = (typeof SECTIONS)[number]["key"] | "merged";

/* Mine and All start collapsed: they are ambient context, not act-now queues,
   so the popover opens on what needs attention (Participated, Merged). */
export const DEFAULT_COLLAPSED: Record<SectionKey, boolean> = {
  mine: true,
  participated: false,
  all: true,
  merged: false,
};

const COLLAPSED_STORAGE_KEY = "collapsed-sections";

// The one loader for persisted collapse state: every failure path, storage
// throwing included, recovers through parseCollapsed.
function loadCollapsed(): Record<SectionKey, boolean> {
  try {
    return parseCollapsed(localStorage.getItem(COLLAPSED_STORAGE_KEY));
  } catch {
    return parseCollapsed(null);
  }
}

// The write half of loadCollapsed's pair; kept beside it so a second writer
// cannot forget the key or the failure handling.
function saveCollapsed(state: Record<SectionKey, boolean>): void {
  try {
    localStorage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // After a failed write the toggle lives only until this component next
    // remounts and re-reads the stale store (a Settings visit or a no-match
    // filter); acceptable for the exotic failures a persistent WKWebView
    // can hit.
  }
}

// Turns the persisted blob back into collapse state. Unknown keys and
// non-boolean values fall back to the defaults, so a stale or corrupt blob
// can never poison the state shape.
export function parseCollapsed(raw: string | null): Record<SectionKey, boolean> {
  const state = { ...DEFAULT_COLLAPSED };
  if (!raw) return state;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return state;
    for (const key of Object.keys(state) as SectionKey[]) {
      const value = (parsed as Record<string, unknown>)[key];
      if (typeof value === "boolean") state[key] = value;
    }
  } catch {
    // Corrupt JSON: keep the defaults.
  }
  return state;
}

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

// The fields a filter term can match: title, repo, "#number", and author.
// Only PullRequest and MergedPr flow through here, and both carry these.
type Filterable = Pick<PullRequest, "title" | "repo" | "author" | "number">;

const queryTerms = (query: string) =>
  query.toLowerCase().split(/\s+/).filter(Boolean);

// True when the query carries at least one searchable term. The single owner
// of "is a filter active": it shares matchesFilter's tokenization, so it can
// never disagree with "an empty query matches everything".
export function hasQuery(query: string): boolean {
  return queryTerms(query).length > 0;
}

// True when every whitespace-separated term appears somewhere in the row's
// searchable text (case-insensitive). Terms are AND'd, so "acme fix" narrows
// rather than widens. An empty or whitespace-only query matches everything.
export function matchesFilter(item: Filterable, query: string): boolean {
  const terms = queryTerms(query);
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

// The round author avatar with badged corners, app-icon style: the face says
// who, the bottom-right square (the repo owner's avatar) says which org, and
// the top-right red counter says how much is new. Opposite corners keep the
// two badges from ever colliding. The org badge has no placeholder by design —
// a missing or broken owner avatar simply shows no badge, keeping the corner
// clean rather than grey.
function PrAvatar({
  avatarUrl,
  ownerAvatarUrl,
  author,
  unreadCount = 0,
}: {
  avatarUrl: string;
  ownerAvatarUrl: string;
  author: string;
  unreadCount?: number;
}) {
  return (
    <span
      /* The author's name lives here as a hover reveal, not in the metadata
         line: the face already says who, so spelling the handle out on every
         row bought little. pr-tip-right because the avatar sits at the card's
         left edge, where the default leftward bubble would clip. */
      className="pr-avatar-wrap pr-tip pr-tip-right"
      role="img"
      data-tip={`@${author}`}
      aria-label={`@${author}`}
    >
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
      {unreadCount > 0 && <span className="pr-unread">{unreadCount}</span>}
    </span>
  );
}

// A grey glyph in the row's right-edge marker cluster: a 13px stroke icon
// whose meaning lives in the hover tooltip and aria-label, both fed by `tip`.
// `wide` lets it hold a direction arrow beside the glasses. The draft and
// blocked pills are not these: they carry their own styling, not a glyph.
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
  const pills = blockedPills(pr.blocked_reasons);
  // The +N pill's tooltip: the reasons its count stands for.
  const overflowTitle = blockedTitle(pr.blocked_reasons.slice(1));
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
        <PrAvatar
          avatarUrl={pr.avatar_url}
          ownerAvatarUrl={pr.owner_avatar_url}
          author={pr.author}
          unreadCount={pr.unread_count}
        />
        <span className="pr-text">
          <span className="pr-title-row">
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">{formatUpdated(pr.updated_at)}</span>
          </span>
          <span className="pr-origin">
            {/* Number first, in its own non-shrinking span, so "PR 510" is
                findable at a fixed x-position on every row: the repo and
                author give way to truncation, the number never does. */}
            <span className="pr-number">#{pr.number}</span>
            {showRepo && <span className="pr-repo">{pr.repo}</span>}
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
                    grey mark like the draft pill, not a blocked signal: an
                    invitation to act, outside the blocked pill's "something is
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
                {pills.length > 0 && (
                  <span className="pr-blocked-pill">{pills[0]}</span>
                )}
                {pills.length > 1 && (
                  <span
                    className="pr-blocked-pill pr-tip"
                    role="img"
                    data-tip={overflowTitle}
                    aria-label={overflowTitle}
                  >
                    {pills[1]}
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
        <PrAvatar
          avatarUrl={pr.avatar_url}
          ownerAvatarUrl={pr.owner_avatar_url}
          author={pr.author}
        />
        <span className="pr-text">
          <span className="pr-title-row">
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">{formatUpdated(pr.merged_at)}</span>
          </span>
          <span className="pr-origin">
            <span className="pr-number">#{pr.number}</span>
            <span className="pr-repo">{pr.repo}</span>
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

/* A section heading that is one whole-row disclosure button, Finder-style:
   chevron, label, and count share a single click target instead of a small
   icon. The h2 keeps the heading in the accessibility outline as a real box
   (not display: contents, which older WebKit drops from the AX tree); the
   button fills it, and trailing siblings (Merged's "Clear all") stay outside
   the toggle and out of nested-button territory. */
function SectionHeader({
  label,
  count,
  expanded,
  onToggle,
  disabled = false,
  unread = 0,
  children,
}: {
  label: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  /** Suspends toggling (used while a filter force-expands every section);
   * aria-disabled rather than disabled so the header stays focusable and
   * announced to assistive tech while inert. */
  disabled?: boolean;
  /** Unread total inside the section; shown as a red badge only while
   * collapsed, when the rows carrying the per-PR badges are hidden. */
  unread?: number;
  children?: ReactNode;
}) {
  return (
    <div className="pr-section-header">
      <h2 className="pr-section-heading">
        <button
          type="button"
          className="pr-section-toggle"
          aria-expanded={expanded}
          aria-disabled={disabled}
          onClick={disabled ? undefined : onToggle}
        >
          <svg
            className="pr-section-chevron"
            viewBox="0 0 24 24"
            width="10"
            height="10"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M9 5l7 7-7 7" />
          </svg>
          <span className="pr-section-label">{label}</span>
          <span className="pr-section-count" aria-hidden="true">
            {count}
          </span>
          {/* The tray badge counts these rows even when the section hides
              them; without this cue a collapsed Mine makes the popover look
              read while the tray says otherwise. */}
          {!expanded && unread > 0 && (
            <span className="pr-section-unread" aria-hidden="true">
              {unread}
            </span>
          )}
          {/* The pills are bare numbers whose meaning is carried by colour,
              so the spoken name gets a worded equivalent instead. */}
          <span className="visually-hidden">
            {`${count} pull request${count === 1 ? "" : "s"}` +
              (!expanded && unread > 0 ? `, ${unread} unread` : "")}
          </span>
        </button>
      </h2>
      {children}
    </div>
  );
}

function PrList({
  prs,
  merged,
  filtering = false,
}: {
  prs: PullRequest[];
  merged: MergedPr[];
  /** True while the header filter has a query. Filtering force-expands every
   * section (a match hidden inside a collapsed section would read as "no
   * match") and suspends toggling; the stored state returns untouched when
   * the query clears. */
  filtering?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const isExpanded = (key: SectionKey) => filtering || !collapsed[key];
  const toggle = (key: SectionKey) => {
    // aria-disabled buttons still fire clicks, so the suspend-while-filtering
    // rule is enforced here, not by the button.
    if (filtering) return;
    const next = { ...collapsed, [key]: !collapsed[key] };
    setCollapsed(next);
    // Persisted outside the state updater: StrictMode double-invokes updaters.
    saveCollapsed(next);
  };
  return (
    <main className="pr-list">
      {SECTIONS.map(({ key, label }) => {
        const rows = prs.filter((pr) => pr.section === key);
        /* The three PR sections always render, empty or not, so the popover
           keeps one stable scaffold (an absent "Mine" would read as broken,
           not empty). Filtering is the exception: rows are pre-filtered, so
           an empty section there means "no matches here" and showing it
           would clutter the force-expanded results. */
        if (rows.length === 0 && filtering) return null;
        let body: ReactNode;
        if (rows.length === 0) {
          /* Inside the slab, not bare text, so an empty section keeps the
             shape rows will appear in. */
          body = (
            <ul>
              <li className="pr-empty">Nothing to wine about</li>
            </ul>
          );
        } else if (key === "all") {
          body = groupByRepo(rows).map(({ repo, prs: group }) => (
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
          ));
        } else {
          body = (
            <ul>
              {rows.map((pr) => (
                <PrRow key={`${pr.repo}#${pr.number}`} pr={pr} />
              ))}
            </ul>
          );
        }
        return (
          <section key={key} className="pr-section">
            <SectionHeader
              label={label}
              count={rows.length}
              expanded={isExpanded(key)}
              onToggle={() => toggle(key)}
              disabled={filtering}
              unread={totalUnread(rows)}
            />
            {isExpanded(key) && body}
          </section>
        );
      })}
      {merged.length > 0 && (
        <section className="pr-section">
          <SectionHeader
            label="Merged"
            count={merged.length}
            expanded={isExpanded("merged")}
            onToggle={() => toggle("merged")}
            disabled={filtering}
          >
            <button
              type="button"
              className="pr-section-clear"
              onClick={() => invoke("clear_merged").catch(() => {})}
            >
              Clear all
            </button>
          </SectionHeader>
          {isExpanded("merged") && (
            <ul>
              {merged.map((pr) => (
                <MergedRow key={`${pr.repo}#${pr.number}`} pr={pr} />
              ))}
            </ul>
          )}
        </section>
      )}
    </main>
  );
}

export default PrList;
