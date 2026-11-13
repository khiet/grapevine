// What the popover shows, derived from the Snapshot, the filter query and the
// collapse state: the placeholder choice, which sections exist and what they
// hold, and each row's state. App, PrList, SectionHeader and PrRow render the
// result; a rule about what appears belongs here, where a test can reach it.
// Pure: no React, no storage, no clock; the caller passes `now`.

import {
  prKey,
  type BlockedReason,
  type Deploy,
  type DeployState,
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

/** The pinned section of rows waiting on the viewer. Not a SectionKey: it is
 * always open, so it has no collapse state to store. */
export const YOUR_TURN_KEY = "yours";

/** The keys of the PRs whose request for a Copilot fix is on its way to
 * GitHub. Held by fixConflict.ts. A request that got there is the backend's
 * to remember: it comes back as the PR's fix_requested. */
export type FixesPosting = ReadonlySet<string>;

/** What colours a row's reason: the accent for the viewer's turn, amber for
 * blocked, green for ready and approved, neutral for everything else. */
export type Tone = "accent" | "warn" | "ok" | "neutral";

/** A row's states, in words. A row shows `reason` and `more`; the rest is
 * for the tooltip and the accessible name. */
export interface RowState {
  /** The highest-priority state. On a your-turn row, the one the backend
   * decided the turn on. */
  reason: string;
  tone: Tone;
  /** "+N" for the N states folded behind the reason; null when it stands
   * alone. */
  more: string | null;
  /** Every state in priority order, the reason first. */
  all: string[];
}

export interface RowView {
  pr: PullRequest;
  /** The neutral Draft pill, shown beside `state` and never part of it. */
  draft: boolean;
  /** Null when the PR has no state to report, so a quiet row keeps meaning
   * "nothing is stuck". */
  state: RowState | null;
  /** The "bot" tag on the avatar: a machine account opened the PR. */
  botAuthor: boolean;
  /** The muted metadata note counting new bot activity; null when there is
   * none. The quiet counterpart of the unread badge. */
  botNote: string | null;
  /** How long the row has waited on the viewer ("waiting 3h", "waiting 2d"),
   * or "waiting" when the start is unknown, shown in place of the updated
   * time. Null when the turn is not the viewer's; the row keeps the updated
   * time then. */
  waiting: string | null;
  /** The row's snooze control: "offer" to park it, "wake" to bring a snoozed
   * row back, null where rows cannot be snoozed (All). */
  snooze: "offer" | "wake" | null;
  /** The row's Fix with Copilot control: "offer" to ask Copilot to resolve
   * the merge conflict, "posting" while the request is on its way, null
   * where the fix is not on offer. A request that was made is a state, not a
   * control. */
  fix: "offer" | "posting" | null;
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

/** The deploy status of a Merged row: its own control, which opens the
 * workflow run rather than the PR. */
export interface DeployView {
  label: string;
  /** Whether the row shows `completedAt` beside the label; only a deploy
   * that landed does. */
  showsTime: boolean;
  /** The one state in the failure color; the rest stay metadata grey. */
  failed: boolean;
  /** When the run finished (ISO-8601 UTC); null while deploying. */
  completedAt: string | null;
  url: string;
}

export interface MergedRowView {
  pr: MergedPr;
  /** Null when no deploy run has been found: the row then shows nothing
   * about deploys at all. */
  deploy: DeployView | null;
}

export type SectionBody =
  /** No rows: the section shows its empty line. */
  | { kind: "empty" }
  /** `rows` is empty when every row of the section is snoozed. */
  | { kind: "rows"; rows: RowView[]; snoozed: SnoozedGroupView | null }
  | { kind: "groups"; groups: RepoGroupView[] }
  | { kind: "merged"; rows: MergedRowView[] };

export interface SectionView {
  /** YOUR_TURN_KEY marks the one section with no collapse toggle. */
  key: SectionKey | typeof YOUR_TURN_KEY;
  label: string;
  count: number;
  /** The body is derived either way; it is shown only while expanded. Always
   * true for Todo. */
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
// one, then "and more" for any others. Never a count: on a truncated list the
// number seen is not the number failing, and "and more" is true either way.
// Worded, not "+ more", so it cannot be read as the row's "+N".
function blockedLabel(reason: BlockedReason, pr: PullRequest): string {
  if (reason !== "ci" || pr.failing_checks.length === 0) {
    return BLOCKED_LABELS[reason];
  }
  const more = pr.failing_checks.length > 1 || pr.failing_checks_truncated;
  return `CI failing: ${pr.failing_checks[0]}${more ? " and more" : ""}`;
}

// English ordinal for a positive integer. The suffix follows the last digit,
// except that 11, 12 and 13 (in every hundred) take "th".
function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13;
  const suffix = teen ? "th" : (["th", "st", "nd", "rd"][n % 10] ?? "th");
  return `${n}${suffix}`;
}

// Every state the PR is in, in priority order; the row shows the first and
// folds the rest. A new signal needs one line here, placed where it ranks.
//
// The head of the order (a review asked of you, then the blockers in the
// backend's severity order, then ready to merge) is the order the backend
// decides the turn in, so on a your-turn row the first state is the reason
// the row sits in Todo. After it the order runs from under way (queue,
// Copilot, checks, auto-merge) to done (approved) to waiting on others.
//
// Drafts need no special case: the backend suppresses everything but the
// queue position on a draft, so whatever it does send shows. Ready is the
// backend's call as well and does not imply approved, since a repo with no
// required reviews is ready without an approval.
function rowStates(pr: PullRequest): { text: string; tone: Tone }[] {
  const states: { text: string; tone: Tone }[] = [];
  const add = (text: string, tone: Tone = "neutral") => states.push({ text, tone });
  // The accent is the viewer's turn, not the request: the same words on a row
  // that does not wait on them are plain news.
  if (pr.review_requested) {
    add("Review requested", pr.turn === "yours" ? "accent" : "neutral");
  }
  for (const reason of pr.blocked_reasons) add(blockedLabel(reason, pr), "warn");
  if (pr.ready_to_merge) add("Ready to merge", "ok");
  // Position only: the entry's state and the queue's size are deliberately
  // not shown.
  if (pr.merge_queue_position !== null) {
    add(`In merge queue, ${ordinal(pr.merge_queue_position)}`);
  }
  // After the conflict it answers: the PR is still conflicting, and this
  // says who is on it.
  if (pr.fix_requested) add("Copilot fixing");
  if (pr.checks_running) add("Checks running");
  if (pr.auto_merge) add("Auto-merge armed");
  if (pr.approved) add("Approved", "ok");
  // The backend sends at most one of the three review directions; the chain
  // holds the row to one regardless, a request naming you first.
  if (pr.awaiting_review) add("Awaiting review");
  else if (pr.team_review_requested && !pr.review_requested) {
    add("Team review requested");
  }
  return states;
}

function rowState(pr: PullRequest): RowState | null {
  const [first, ...rest] = rowStates(pr);
  if (!first) return null;
  return {
    reason: first.text,
    tone: first.tone,
    more: rest.length > 0 ? `+${rest.length}` : null,
    all: [first.text, ...rest.map((state) => state.text)],
  };
}

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

// Whole hours under a day, whole days from one day up, both rounded down: a
// wait is never overstated. Under an hour reads "<1h" rather than "0h". A
// turn nobody watched begin has no age to show, and says "waiting" so the
// row still carries the cue.
function waitingLabel(pr: PullRequest, now: number): string | null {
  if (pr.turn !== "yours") return null;
  if (pr.turn_since === null) return "waiting";
  const elapsed = now - pr.turn_since;
  if (elapsed < HOUR_MS) return "waiting <1h";
  if (elapsed < DAY_MS) return `waiting ${Math.floor(elapsed / HOUR_MS)}h`;
  return `waiting ${Math.floor(elapsed / DAY_MS)}d`;
}

// Offered wherever the backend allows it, except on a row whose fix was
// already requested, and on a snoozed row: that one was put out of the way,
// and waking it is the only thing it offers. A request on its way shows in
// the offer's place.
function fixControl(
  pr: PullRequest,
  snooze: RowView["snooze"],
  fixesPosting: FixesPosting,
): RowView["fix"] {
  if (!pr.can_fix_conflict || pr.fix_requested || snooze === "wake") return null;
  return fixesPosting.has(prKey(pr)) ? "posting" : "offer";
}

const rowView = (
  pr: PullRequest,
  now: number,
  snooze: RowView["snooze"],
  fixesPosting: FixesPosting,
): RowView => ({
  pr,
  draft: pr.is_draft,
  state: rowState(pr),
  botAuthor: pr.author_is_bot,
  botNote:
    pr.bot_unread_count > 0 ? `${pr.bot_unread_count} new from bots` : null,
  waiting: waitingLabel(pr, now),
  snooze,
  fix: fixControl(pr, snooze, fixesPosting),
});

// Rows asking the viewer's team lead a section, then the rest. Rows waiting
// on the viewer are not here to rank: they sit in Todo.
// Array.prototype.sort is stable, so each group keeps the order the backend
// sent.
function teamRequestsFirst(prs: PullRequest[]): PullRequest[] {
  const rank = (pr: PullRequest) => (pr.team_review_requested ? 0 : 1);
  return [...prs].sort((a, b) => rank(a) - rank(b));
}

/** The unread total across PRs, as badged on a collapsed section. Bot
 * activity is not in it, and it is not the tray number: see yourTurnCount. */
function totalUnread(prs: PullRequest[]): number {
  return prs.reduce((sum, pr) => sum + pr.unread_count, 0);
}

// The one membership rule shared by the Todo section and yourTurnCount,
// so the two cannot disagree: the row waits on the viewer and is not snoozed.
// The backend counts the tray number by the same rule.
const isYourTurn = (pr: PullRequest, snoozedKeys: ReadonlySet<string>) =>
  pr.turn === "yours" && !snoozedKeys.has(prKey(pr));

/** How many PRs wait on the viewer and are not snoozed; the tray shows the
 * same number, and so does the Todo section while no filter is active. */
export function yourTurnCount(snapshot: Snapshot): number {
  const snoozedKeys = new Set(snapshot.snoozed);
  return snapshot.prs.filter((pr) => isYourTurn(pr, snoozedKeys)).length;
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
function groupByRepo(
  prs: PullRequest[],
  now: number,
  fixesPosting: FixesPosting,
): RepoGroupView[] {
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
      rows: rows.map((pr) => rowView(pr, now, null, fixesPosting)),
    }));
}

const DEPLOY_LABELS: Record<DeployState, string> = {
  deploying: "Deploying",
  deployed: "Deployed",
  failed: "Deploy failed",
  cancelled: "Deploy cancelled",
};

function deployView(deploy: Deploy | null): DeployView | null {
  if (!deploy) return null;
  return {
    label: DEPLOY_LABELS[deploy.state],
    showsTime: deploy.state === "deployed",
    failed: deploy.state === "failed",
    completedAt: deploy.completed_at,
    url: deploy.url,
  };
}

export function derivePopover(
  snapshot: Snapshot,
  query: string,
  collapsed: Record<SectionKey, boolean>,
  /** Epoch ms; what the waiting labels are measured against. */
  now: number,
  /** The Copilot fixes being asked for right now; none when omitted. */
  fixesPosting: FixesPosting = new Set(),
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
  // Todo takes its rows out of their home sections, so a PR is listed
  // and counted once. A snoozed row is not taken: it stays in its home
  // section's fold. The rows keep the order the backend sent.
  const yours = prs.filter((pr) => isYourTurn(pr, snoozedKeys));
  const yoursSet = new Set(yours);
  // Pinned and always open, empty or not: an absent Todo would leave the
  // tray's zero with nothing to match. Hidden only by a filter with no match
  // here, like every other section.
  if (yours.length > 0 || !filtering) {
    sections.push({
      key: YOUR_TURN_KEY,
      label: "Todo",
      count: yours.length,
      expanded: true,
      collapsedUnread: 0,
      offerClearAll: false,
      body:
        yours.length === 0
          ? { kind: "empty" }
          : {
              kind: "rows",
              // A row keeps the snooze control of its home section.
              rows: yours.map((pr) =>
                rowView(pr, now, pr.section === "all" ? null : "offer", fixesPosting),
              ),
              snoozed: null,
            },
    });
  }
  for (const { key, label } of SECTIONS) {
    const rows = prs.filter((pr) => pr.section === key && !yoursSet.has(pr));
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
      body = { kind: "groups", groups: groupByRepo(rows, now, fixesPosting) };
    } else {
      body = {
        kind: "rows",
        rows: teamRequestsFirst(awake).map((pr) => rowView(pr, now, "offer", fixesPosting)),
        snoozed:
          snoozed.length === 0
            ? null
            : {
                label: `${snoozed.length} snoozed`,
                rows: snoozed.map((pr) => rowView(pr, now, "wake", fixesPosting)),
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
         counts the rows of Todo, so the two can disagree. */
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
      body: {
        kind: "merged",
        rows: merged.map((pr) => ({ pr, deploy: deployView(pr.deploy) })),
      },
    });
  }

  return {
    offerMarkAllRead,
    content: { kind: "list", sections, canToggle: !filtering },
  };
}
