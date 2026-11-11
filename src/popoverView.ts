// What the popover shows, derived from the Snapshot, the filter query and the
// collapse state: the placeholder choice, which sections exist and what they
// hold, and each row's marks. App, PrList, SectionHeader and PrRow render the
// result; a rule about what appears belongs here, where a test can reach it.
// Pure: no React, no storage, no clock; the caller passes `now`.

import {
  prKey,
  type BlockedReason,
  type MergedPr,
  type PullRequest,
  type Snapshot,
} from "./backend";

const SECTIONS = [
  { key: "mine", label: "Mine" },
  { key: "participated", label: "Participated" },
  { key: "all", label: "All" },
] as const;

/** Every collapsible top-level section; repo sub-groups inside "All" are
 * deliberately not collapsible. */
export type SectionKey = (typeof SECTIONS)[number]["key"] | "merged";

/** One item of a row's right-edge marker cluster. `tip` is the hover tooltip
 * and the accessible name; null on the primary blocked pill, which is its own
 * label, and absent on the ready pill for the same reason. `yourTurn` is the
 * tint flag: set on the one mark that says why the row waits on the viewer,
 * and absent everywhere else. */
export type Mark =
  | { kind: "approved"; tip: string }
  | { kind: "review-requested"; tip: string; yourTurn?: true }
  | { kind: "team-review-requested"; tip: string }
  | { kind: "awaiting-review"; tip: string }
  | { kind: "merge-queue"; position: number; tip: string }
  | { kind: "ready"; label: string; yourTurn?: true }
  | { kind: "auto-merge"; label: string; tip: string }
  | { kind: "checks-running"; label: string; tip: string }
  | { kind: "blocked"; label: string; tip: string | null; yourTurn?: true };

export interface RowView {
  pr: PullRequest;
  /** The neutral Draft pill; a state, not a mark. */
  draft: boolean;
  /** In display order. Empty means the row has no marker cluster. */
  marks: Mark[];
  /** The "bot" tag on the avatar: a machine account opened the PR. */
  botAuthor: boolean;
  /** The muted metadata note counting new bot activity; null when there is
   * none. The quiet counterpart of the unread badge. */
  botNote: string | null;
  /** How long the row has waited on the viewer ("waiting 3h", "waiting 2d"),
   * shown in place of the updated time. Null when the turn is not the
   * viewer's or its start is unknown; the row keeps the updated time then. */
  waiting: string | null;
  /** The row's snooze control: "offer" to park it, "wake" to bring a snoozed
   * row back, null where rows cannot be snoozed (All). */
  snooze: "offer" | "wake" | null;
}

/** The snoozed rows of a section, folded into one line at its foot. */
export interface SnoozedGroupView {
  /** The folded line, e.g. "2 snoozed". */
  label: string;
  rows: RowView[];
  /** True while a filter is active: the rows are matches, so they show
   * whether or not the line was opened by hand. */
  forcedOpen: boolean;
}

export interface RepoGroupView {
  repo: string;
  rows: RowView[];
}

export type SectionBody =
  /** No rows: the section shows its empty line. */
  | { kind: "empty" }
  /** `rows` is empty when every row of the section is snoozed. */
  | { kind: "rows"; rows: RowView[]; snoozed: SnoozedGroupView | null }
  | { kind: "groups"; groups: RepoGroupView[] }
  | { kind: "merged"; rows: MergedPr[] };

export interface SectionView {
  key: SectionKey;
  label: string;
  count: number;
  /** The body is derived either way; it is shown only while expanded. */
  expanded: boolean;
  /** Unread total to badge on the header: zero while expanded, when the rows
   * carry their own badges. */
  collapsedUnread: number;
  /** Whether the header offers Clear all. */
  offerClearAll: boolean;
  body: SectionBody;
}

export interface ListView {
  kind: "list";
  sections: SectionView[];
  /** False while a filter force-expands every section; the stored collapse
   * state then returns untouched when the query clears. */
  canToggle: boolean;
}

export type PopoverContent =
  /** Nothing synced and nothing to show: prompt for a token and repos. */
  | { kind: "setup" }
  /** A filter is active and hid every row. */
  | { kind: "no-matches"; query: string }
  | ListView;

export interface PopoverView {
  /** Whether the header offers Mark all read. */
  offerMarkAllRead: boolean;
  content: PopoverContent;
}

/* Neutral PR-property wording, deliberately not "you must act": the section
   the row sits in supplies the "is this mine to fix?" context. */
const BLOCKED_LABELS: Record<BlockedReason, string> = {
  conflict: "Merge conflict",
  ci: "CI failing",
  review: "Changes requested",
  threads: "Unresolved threads",
  behind: "Behind base",
};

// The CI reason names the check when the backend could: the first failing
// one, then "+ more" for any others. Never a count: on a truncated list the
// number seen is not the number failing, and "+ more" is true either way.
function blockedLabel(reason: BlockedReason, pr: PullRequest): string {
  if (reason !== "ci" || pr.failing_checks.length === 0) {
    return BLOCKED_LABELS[reason];
  }
  const more = pr.failing_checks.length > 1 || pr.failing_checks_truncated;
  return `CI: ${pr.failing_checks[0]}${more ? " + more" : ""}`;
}

// The blocked pills: the first reason spelled out, then "+N" for the rest, so
// a doubly-blocked row stays width-bounded ("Merge conflict" "+1") instead of
// listing everything inline. The first reason wins because the backend orders
// by severity (conflict, ci, review, threads, behind). The +N pill's tooltip
// names the reasons its count stands for.
function blockedMarks(pr: PullRequest): Mark[] {
  if (pr.blocked_reasons.length === 0) return [];
  const [first, ...rest] = pr.blocked_reasons;
  const marks: Mark[] = [
    { kind: "blocked", label: blockedLabel(first, pr), tip: null },
  ];
  if (rest.length > 0) {
    marks.push({
      kind: "blocked",
      label: `+${rest.length}`,
      tip: rest.map((reason) => blockedLabel(reason, pr)).join("; "),
    });
  }
  return marks;
}

// English ordinal for a positive integer. The suffix follows the last digit,
// except that 11, 12 and 13 (in every hundred) take "th".
function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13;
  const suffix = teen ? "th" : (["th", "st", "nd", "rd"][n % 10] ?? "th");
  return `${n}${suffix}`;
}

// The marker cluster in display order: the approved check, the ready pill,
// the review glyph (incoming, incoming through a team, or outgoing; the
// backend sends at most one for a row, and the chain below holds the slot to
// one mark regardless, a request naming you first), the merge marks (queue
// position, auto-merge, checks running), then the blocked pills. The positive
// marks lead, so they read before whatever still stands in the way, and the
// order runs from done, through under way, to stuck. A new signal needs a
// Mark kind, a line here for when and where it shows, and its glyph in
// MarkView; no condition elsewhere has to learn about it.
//
// Drafts need no special case: the backend suppresses the check, the review
// glyphs, the merge marks other than the queue position and the blocked
// reasons on a draft, so whatever it does send shows. Ready is the backend's
// call as well: the pill does not imply the check, since a repo with no
// required reviews is ready without an approval.
function rowMarks(pr: PullRequest): Mark[] {
  const marks: Mark[] = [];
  if (pr.approved) marks.push({ kind: "approved", tip: "Approved" });
  if (pr.ready_to_merge) marks.push({ kind: "ready", label: "Ready to merge" });
  if (pr.review_requested) {
    marks.push({ kind: "review-requested", tip: "Review requested" });
  } else if (pr.team_review_requested) {
    marks.push({
      kind: "team-review-requested",
      tip: "Review requested from your team",
    });
  } else if (pr.awaiting_review) {
    marks.push({ kind: "awaiting-review", tip: "Awaiting review" });
  }
  if (pr.merge_queue_position !== null) {
    // Position only: the entry's state and the queue's size are deliberately
    // not shown.
    marks.push({
      kind: "merge-queue",
      position: pr.merge_queue_position,
      tip: `${ordinal(pr.merge_queue_position)} in merge queue`,
    });
  }
  if (pr.auto_merge) {
    marks.push({ kind: "auto-merge", label: "Auto", tip: "Auto-merge armed" });
  }
  if (pr.checks_running) {
    marks.push({
      kind: "checks-running",
      label: "Running",
      tip: "Checks running",
    });
  }
  marks.push(...blockedMarks(pr));
  if (pr.turn === "yours") tintYourTurn(marks);
  return marks;
}

// Flags the mark that explains a your-turn row: the review glyph when a
// review is asked of you, else the primary blocked pill, else the ready pill.
// That is the order the backend decides the turn in, so the tinted mark is
// the reason the row leads. At most one mark is flagged; the +N pill never
// is, since the primary pill already speaks for the blockage.
function tintYourTurn(marks: Mark[]): void {
  for (const kind of ["review-requested", "blocked", "ready"] as const) {
    const mark = marks.find((candidate) => candidate.kind === kind);
    if (mark?.kind === kind) {
      mark.yourTurn = true;
      return;
    }
  }
}

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

// Whole hours under a day, whole days from one day up, both rounded down: a
// wait is never overstated. Under an hour reads "<1h" rather than "0h". The
// backend sends a start only on a your-turn row, so the start alone decides.
function waitingLabel(pr: PullRequest, now: number): string | null {
  if (pr.turn_since === null) return null;
  const elapsed = now - pr.turn_since;
  if (elapsed < HOUR_MS) return "waiting <1h";
  if (elapsed < DAY_MS) return `waiting ${Math.floor(elapsed / HOUR_MS)}h`;
  return `waiting ${Math.floor(elapsed / DAY_MS)}d`;
}

const rowView = (
  pr: PullRequest,
  now: number,
  snooze: RowView["snooze"],
): RowView => ({
  pr,
  draft: pr.is_draft,
  marks: rowMarks(pr),
  botAuthor: pr.author_is_bot,
  botNote:
    pr.bot_unread_count > 0 ? `${pr.bot_unread_count} new from bots` : null,
  waiting: waitingLabel(pr, now),
  snooze,
});

// Rows waiting on the viewer lead a section, then the ones asking the viewer's
// team, then the rest. Array.prototype.sort is stable, so each group keeps
// the order the backend sent.
function byTurn(prs: PullRequest[]): PullRequest[] {
  const rank = (pr: PullRequest) =>
    pr.turn === "yours" ? 0 : pr.team_review_requested ? 1 : 2;
  return [...prs].sort((a, b) => rank(a) - rank(b));
}

/** The unread total across PRs, as badged on a collapsed section. Bot
 * activity is not in it, and it is not the tray number: see yourTurnCount. */
function totalUnread(prs: PullRequest[]): number {
  return prs.reduce((sum, pr) => sum + pr.unread_count, 0);
}

/** How many PRs wait on the viewer and are not snoozed; the tray shows the
 * same number. */
export function yourTurnCount(snapshot: Snapshot): number {
  return snapshot.prs.filter(
    (pr) => pr.turn === "yours" && !snapshot.snoozed.includes(prKey(pr)),
  ).length;
}

// The fields a filter term can match: title, repo, "#number", and author.
// Only PullRequest and MergedPr flow through here, and both carry these.
type Filterable = Pick<PullRequest, "title" | "repo" | "author" | "number">;

const queryTerms = (query: string) =>
  query.toLowerCase().split(/\s+/).filter(Boolean);

// True when every term appears somewhere in the row's searchable text
// (case-insensitive). Terms are AND'd, so "acme fix" narrows rather than
// widens. No terms matches everything; "is a filter active" is asked of the
// same term list, so the two can never disagree.
function matchesTerms(item: Filterable, terms: string[]): boolean {
  const haystack =
    `${item.title} ${item.repo} #${item.number} ${item.author}`.toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

// Groups ordered by their most recently updated PR, so the repo that just moved
// leads; within a group the incoming order is preserved. An unparseable
// updated_at sorts as the epoch rather than throwing.
function groupByRepo(prs: PullRequest[], now: number): RepoGroupView[] {
  const groups = new Map<string, PullRequest[]>();
  for (const pr of prs) {
    const existing = groups.get(pr.repo);
    if (existing) existing.push(pr);
    else groups.set(pr.repo, [pr]);
  }
  const latest = (rows: PullRequest[]) =>
    rows.reduce((max, pr) => Math.max(max, Date.parse(pr.updated_at) || 0), 0);
  return [...groups.entries()]
    .sort(([, a], [, b]) => latest(b) - latest(a))
    .map(([repo, rows]) => ({
      repo,
      rows: rows.map((pr) => rowView(pr, now, null)),
    }));
}

export function derivePopover(
  snapshot: Snapshot,
  query: string,
  collapsed: Record<SectionKey, boolean>,
  /** Epoch ms; what the waiting labels are measured against. */
  now: number,
): PopoverView {
  // Counts every PR, filtered out or not: the action clears them all. A bot
  // note counts too, since the action is the one way to clear every note at
  // once.
  const offerMarkAllRead = snapshot.prs.some(
    (pr) => pr.unread_count > 0 || pr.bot_unread_count > 0,
  );

  // Pre-setup only; once a sync has happened the section scaffold owns the
  // empty state.
  const hasAny = snapshot.prs.length > 0 || snapshot.merged.length > 0;
  if (!snapshot.has_synced && !hasAny) {
    return { offerMarkAllRead, content: { kind: "setup" } };
  }

  const terms = queryTerms(query);
  const filtering = terms.length > 0;
  const prs = snapshot.prs.filter((pr) => matchesTerms(pr, terms));
  const merged = snapshot.merged.filter((pr) => matchesTerms(pr, terms));
  if (filtering && prs.length === 0 && merged.length === 0) {
    return { offerMarkAllRead, content: { kind: "no-matches", query } };
  }

  // Filtering force-expands every section: a match hidden inside a collapsed
  // section would read as "no match".
  const isExpanded = (key: SectionKey) => filtering || !collapsed[key];
  const snoozedKeys = new Set(snapshot.snoozed);

  const sections: SectionView[] = [];
  for (const { key, label } of SECTIONS) {
    const rows = prs.filter((pr) => pr.section === key);
    /* The three PR sections are always present, empty or not, so the popover
       keeps one stable scaffold (an absent "Mine" would read as broken, not
       empty). Filtering is the exception: an empty section there means "no
       matches here" and showing it would clutter the force-expanded
       results. */
    if (rows.length === 0 && filtering) continue;
    const expanded = isExpanded(key);
    // The backend snoozes only Mine and Participated rows, so All never
    // folds. A snoozed row gives up its place and its claim on attention,
    // but still counts towards the header: it has not left the section.
    const snoozed =
      key === "all" ? [] : rows.filter((pr) => snoozedKeys.has(prKey(pr)));
    const awake =
      key === "all" ? rows : rows.filter((pr) => !snoozedKeys.has(prKey(pr)));
    let body: SectionBody;
    if (rows.length === 0) body = { kind: "empty" };
    else if (key === "all") {
      body = { kind: "groups", groups: groupByRepo(rows, now) };
    } else {
      body = {
        kind: "rows",
        rows: byTurn(awake).map((pr) => rowView(pr, now, "offer")),
        snoozed:
          snoozed.length === 0
            ? null
            : {
                label: `${snoozed.length} snoozed`,
                rows: snoozed.map((pr) => rowView(pr, now, "wake")),
                // A match folded out of sight would read as "no match".
                forcedOpen: filtering,
              },
      };
    }
    sections.push({
      key,
      label,
      count: rows.length,
      expanded,
      /* Rows the section hides still carry unread; without this a collapsed
         Mine looks read when it is not. Independent of the tray number, which
         counts your-turn rows, so the two can disagree. */
      collapsedUnread: expanded ? 0 : totalUnread(awake),
      offerClearAll: false,
      body,
    });
  }
  // Merged is the one section that goes away when empty. Its rows carry no
  // unread state.
  if (merged.length > 0) {
    const expanded = isExpanded("merged");
    sections.push({
      key: "merged",
      label: "Merged",
      count: merged.length,
      expanded,
      collapsedUnread: 0,
      /* Not while collapsed: clearing rows you cannot see invites a
         destructive misclick from the adjacent whole-row toggle. */
      offerClearAll: expanded,
      body: { kind: "merged", rows: merged },
    });
  }

  return {
    offerMarkAllRead,
    content: { kind: "list", sections, canToggle: !filtering },
  };
}
