import { useState, type ReactNode } from "react";
import {
  clearMerged,
  dismissMerged,
  markRead,
  openUrl,
  prKey,
  type MergedPr,
} from "./backend";
import type {
  ListView,
  Mark,
  RowView,
  SectionKey,
  SectionView,
} from "./popoverView";

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
    // After a failed write the toggle lives only until the app reloads and
    // re-reads the stale store; acceptable for the exotic failures a
    // persistent WKWebView can hit.
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

/** The stored collapse state and its toggle. Call it where the popover view is
 * derived, above anything that unmounts with the list, so the state outlives
 * a Settings visit or a no-match filter. */
export function useCollapsedSections(): [
  Record<SectionKey, boolean>,
  (key: SectionKey) => void,
] {
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const toggle = (key: SectionKey) => {
    const next = { ...collapsed, [key]: !collapsed[key] };
    setCollapsed(next);
    // Persisted outside the state updater: StrictMode double-invokes updaters.
    saveCollapsed(next);
  };
  return [collapsed, toggle];
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

// Footer label for the last successful sync, reusing the row-timestamp
// format: "Synced 14:59" today, then "Synced Yesterday", "Synced 14 Jul".
export function formatLastSync(
  ms: number | null,
  now: Date = new Date(),
): string | null {
  if (ms === null) return null;
  return `Synced ${formatUpdated(new Date(ms).toISOString(), now)}`;
}

// The round author avatar with badged corners, app-icon style: the face says
// who, the bottom-right square (the repo owner's avatar) says which org, and
// the top-right red counter says how much is new. Opposite corners keep the
// two badges from ever colliding. The bottom-left "bot" tag marks a machine
// author. The org badge has no placeholder by design:
// a missing or broken owner avatar simply shows no badge, keeping the corner
// clean rather than grey.
function PrAvatar({
  avatarUrl,
  ownerAvatarUrl,
  author,
  unreadCount = 0,
  bot = false,
}: {
  avatarUrl: string;
  ownerAvatarUrl: string;
  author: string;
  unreadCount?: number;
  bot?: boolean;
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
      aria-label={bot ? `@${author}, bot` : `@${author}`}
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
      {bot && <span className="pr-bot-tag">bot</span>}
      {unreadCount > 0 && <span className="pr-unread">{unreadCount}</span>}
    </span>
  );
}

// A grey glyph in the row's right-edge marker cluster: a 13px stroke icon
// whose meaning lives in the hover tooltip and aria-label, both fed by `tip`.
// `wide` lets it hold a direction arrow beside the glasses; `ok` turns the
// glyph green, the cluster's one positive colour; `count` styles a number set
// beside the glyph (the merge queue position). The draft and blocked pills
// are not these: they carry their own styling, not a glyph.
function RowMark({
  tip,
  wide,
  ok,
  count,
  children,
}: {
  tip: string;
  wide?: boolean;
  ok?: boolean;
  count?: boolean;
  children: ReactNode;
}) {
  const className = [
    "pr-glyph",
    ...(wide ? ["pr-glyph-wide"] : []),
    ...(ok ? ["pr-glyph-ok"] : []),
    ...(count ? ["pr-glyph-count"] : []),
    "pr-tip",
  ].join(" ");
  return (
    <span className={className} role="img" data-tip={tip} aria-label={tip}>
      {children}
    </span>
  );
}

// The shared frame around every stroke glyph: shapes are drawn on a 24-unit
// grid and scaled down to their rendered size. Square at `size` unless
// `width` narrows it (the direction arrow). Hidden from assistive tech in
// all uses: the enclosing element carries the label (RowMark's aria-label,
// the section toggle's text).
function GlyphSvg({
  strokeWidth,
  size = 13,
  width = size,
  className,
  children,
}: {
  strokeWidth: number;
  size?: number;
  width?: number;
  className?: string;
  children: ReactNode;
}) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      width={width}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

// The review glasses, shared by both review-request directions.
function Glasses() {
  return (
    <GlyphSvg strokeWidth={2}>
      <circle cx="6" cy="15" r="4" />
      <circle cx="18" cy="15" r="4" />
      <path d="M14 15a2 2 0 0 0-4 0" />
      <path d="M2.5 13 5 7c.7-1.3 1.4-2 3-2" />
      <path d="M21.5 13 19 7c-.7-1.3-1.5-2-3-2" />
    </GlyphSvg>
  );
}

// The approved check, the marker cluster's one green mark. A hair heavier
// stroke than the glasses so the short segments stay legible at 13px.
function Check() {
  return (
    <GlyphSvg strokeWidth={2.5}>
      <path d="M4 12.5l5 5L20 6.5" />
    </GlyphSvg>
  );
}

// The stack glyph of the merge queue mark.
function Stack() {
  return (
    <GlyphSvg strokeWidth={2}>
      <path d="M12 2 2 7l10 5 10-5-10-5z" />
      <path d="M2 12l10 5 10-5" />
      <path d="M2 17l10 5 10-5" />
    </GlyphSvg>
  );
}

// The direction arrow that pairs with the glasses. "in" points at the viewer
// (someone requested their review); "out" points away (the viewer is waiting
// on a reviewer). A hair heavier stroke than the glasses so it stays legible
// at its narrow width.
function DirectionArrow({ dir }: { dir: "in" | "out" }) {
  return (
    <GlyphSvg strokeWidth={2.4} width={9}>
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
    </GlyphSvg>
  );
}

// One item of the marker cluster. Which marks a row has, and their order, is
// decided by the popover view; this only draws the one it is handed.
function MarkView({ mark }: { mark: Mark }) {
  switch (mark.kind) {
    // The green check: the required approvals are in.
    case "approved":
      return (
        <RowMark tip={mark.tip} ok>
          <Check />
        </RowMark>
      );
    // Glasses with an incoming arrow: your review is requested. A grey mark
    // like the draft pill, not a blocked signal: an invitation to act, outside
    // the blocked pill's "something is stuck" vocabulary.
    case "review-requested":
      return (
        <RowMark tip={mark.tip} wide>
          <DirectionArrow dir="in" />
          <Glasses />
        </RowMark>
      );
    // Glasses with an outgoing arrow: one of your PRs is waiting on a
    // reviewer.
    case "awaiting-review":
      return (
        <RowMark tip={mark.tip} wide>
          <Glasses />
          <DirectionArrow dir="out" />
        </RowMark>
      );
    // The stack with the PR's place in the merge queue. A grey mark like the
    // glasses, since being queued is neither stuck nor a call to act.
    case "merge-queue":
      return (
        <RowMark tip={mark.tip} count>
          <Stack />
          {mark.position}
        </RowMark>
      );
    /* A blocked reason, spelled out in a pill (the Draft pill's shape in the
       blocked orange) so "why is this stuck" reads without hovering. Only the
       +N pill has a tooltip; the primary pill IS its own label. Distinct from
       the red unread badge in hue and meaning: orange here says "this PR is
       blocked", red there says "someone spoke". */
    case "blocked":
      return mark.tip === null ? (
        <span className="pr-blocked-pill">{mark.label}</span>
      ) : (
        <span
          className="pr-blocked-pill pr-tip"
          role="img"
          data-tip={mark.tip}
          aria-label={mark.tip}
        >
          {mark.label}
        </span>
      );
  }
}

// showRepo is false inside a repo group, where the header already names it.
function PrRow({ row, showRepo = true }: { row: RowView; showRepo?: boolean }) {
  const { pr, draft, marks, botAuthor, botNote } = row;
  const unread = pr.unread_count > 0;
  const open = () => {
    openUrl(pr.url).catch(() => {});
    // The backend clears the badge and pushes a fresh snapshot.
    markRead(prKey(pr)).catch(() => {});
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
          bot={botAuthor}
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
            {/* Plain metadata, not a badge: bots spoke, nobody is waiting. */}
            {botNote && <span className="pr-bot-note">{botNote}</span>}
            {/* A neutral state pill, never a signal. */}
            {draft && <span className="pr-draft">Draft</span>}
            {/* The action markers cluster at the row's right edge as one group
                instead of scattering through the metadata. A quiet row has no
                cluster at all, so it keeps meaning "nothing is stuck". */}
            {marks.length > 0 && (
              <span className="pr-marks">
                {marks.map((mark, index) => (
                  // Marks have no identity beyond their slot in the cluster.
                  <MarkView key={index} mark={mark} />
                ))}
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
          dismissMerged(prKey(pr)).catch(() => {})
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
  /** Unread total to show as a red badge; the caller passes zero while the
   * section is expanded and its rows carry the per-PR badges. */
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
          <GlyphSvg strokeWidth={2.5} size={10} className="pr-section-chevron">
            <path d="M9 5l7 7-7 7" />
          </GlyphSvg>
          <span className="pr-section-label">{label}</span>
          <span className="pr-section-count" aria-hidden="true">
            {count}
          </span>
          {unread > 0 && (
            <span className="pr-section-unread" aria-hidden="true">
              {unread}
            </span>
          )}
          {/* The pills are bare numbers whose meaning is carried by colour,
              so the spoken name gets a worded equivalent instead. */}
          <span className="visually-hidden">
            {`${count} pull request${count === 1 ? "" : "s"}` +
              (unread > 0 ? `, ${unread} unread` : "")}
          </span>
        </button>
      </h2>
      {children}
    </div>
  );
}

function SectionBodyView({ section }: { section: SectionView }) {
  const { body } = section;
  switch (body.kind) {
    case "empty":
      /* Inside the slab, not bare text, so an empty section keeps the shape
         rows will appear in. */
      return (
        <ul>
          <li className="pr-empty">Nothing to wine about</li>
        </ul>
      );
    case "rows":
      return (
        <ul>
          {body.rows.map((row) => (
            <PrRow key={prKey(row.pr)} row={row} />
          ))}
        </ul>
      );
    case "groups":
      return body.groups.map(({ repo, rows }) => (
        <div key={repo} className="pr-repo-group">
          <div className="pr-repo-header">
            <h3 className="pr-repo-name">{repo}</h3>
            <span className="pr-repo-count">{rows.length}</span>
          </div>
          <ul>
            {rows.map((row) => (
              <PrRow key={prKey(row.pr)} row={row} showRepo={false} />
            ))}
          </ul>
        </div>
      ));
    case "merged":
      return (
        <ul>
          {body.rows.map((pr) => (
            <MergedRow key={prKey(pr)} pr={pr} />
          ))}
        </ul>
      );
  }
}

// Renders the sections it is given: which exist, what they hold and whether
// they are open is the popover view's call.
function PrList({
  view,
  onToggle,
}: {
  view: ListView;
  onToggle: (key: SectionKey) => void;
}) {
  return (
    <main className="pr-list">
      {view.sections.map((section) => (
        <section key={section.key} className="pr-section">
          <SectionHeader
            label={section.label}
            count={section.count}
            expanded={section.expanded}
            onToggle={() => onToggle(section.key)}
            disabled={!view.canToggle}
            unread={section.collapsedUnread}
          >
            {section.offerClearAll && (
              <button
                type="button"
                className="pr-section-clear"
                onClick={() => clearMerged().catch(() => {})}
              >
                Clear all
              </button>
            )}
          </SectionHeader>
          {section.expanded && <SectionBodyView section={section} />}
        </section>
      ))}
    </main>
  );
}

export default PrList;
