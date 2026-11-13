import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import {
  clearMerged,
  dismissMerged,
  markRead,
  openUrl,
  prKey,
  snooze,
  wake,
} from "./backend";
import { clickFix } from "./fixConflict";
import {
  filterCommand,
  findRow,
  keyHints,
  listCommand,
  moveSelection,
  rowCommand,
  selectionAfter,
  type KeyHint,
  type Zone,
} from "./listKeys";
import RowMenu, { opensMenu, type Point } from "./RowMenu";
import {
  YOUR_TURN_KEY,
  type DeployView,
  type ListView,
  type MergedRowView,
  type RowAction,
  type RowActionId,
  type RowState,
  type RowView,
  type SectionKey,
  type SectionView,
  type SnoozedGroupView,
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

/** What the list and its rows take from the keyboard model. */
export interface ListKeys {
  ref: RefObject<HTMLElement | null>;
  selected: string | null;
  select: (key: string) => void;
  /** A row calls this before running an action by key: if the row leaves
   * the list while it holds focus, focus follows the selection instead of
   * being dropped. */
  acted: () => void;
  onKeyDown: (event: KeyboardEvent) => void;
}

const NO_LIST_KEYS: ListKeys = {
  ref: { current: null },
  selected: null,
  select: () => {},
  acted: () => {},
  onKeyDown: () => {},
};

const ListKeysContext = createContext(NO_LIST_KEYS);

/** The list's keyboard model, applied: a selection the arrows move from the
 * filter without taking focus out of it, and the keys of the list itself.
 * Call it beside the filter field and hand `list` to PrList. `view` is the
 * list as shown, null while something else is.
 *
 * Which rows are shown, and in what order, is read from the rendered list
 * (each row's li carries data-row-key): a snoozed fold opened by hand is
 * state of the list, not of the view. */
export function useListKeys(view: ListView | null): {
  filter: {
    ref: RefObject<HTMLInputElement | null>;
    onKeyDown: (event: KeyboardEvent) => void;
  };
  list: ListKeys;
  /** For an element around both the filter and the list. */
  around: {
    onFocus: (event: FocusEvent) => void;
    onPointerDown: () => void;
  };
  hints: KeyHint[];
  /** The selected row in words, set when the arrows move the selection
   * from the filter; a row that takes focus announces itself. */
  announcement: string;
} {
  const list = useRef<HTMLElement>(null);
  const filter = useRef<HTMLInputElement>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [zone, setZone] = useState<Zone>(null);
  const [announcement, setAnnouncement] = useState("");
  // The rows as last rendered, in order.
  const order = useRef<string[]>([]);
  const refocus = useRef(false);

  const zoneOf = (element: Node | null): Zone => {
    if (element !== null && element === filter.current) return "filter";
    return list.current?.contains(element) ? "list" : null;
  };
  const rowItems = () => [
    ...(list.current?.querySelectorAll<HTMLElement>("[data-row-key]") ?? []),
  ];
  const rowItem = (key: string) =>
    rowItems().find((item) => item.dataset.rowKey === key) ?? null;
  const rowButton = (key: string) =>
    rowItem(key)?.querySelector<HTMLElement>(".pr-row") ?? null;

  // After every render, since anything can change the rows: a sync, the
  // filter, a section or a fold toggled.
  useLayoutEffect(() => {
    const keys = rowItems().map((item) => item.dataset.rowKey as string);
    const next = selectionAfter(order.current, keys, selected);
    order.current = keys;
    if (next !== selected) {
      setSelected(next);
      return;
    }
    // Read here as well as in onFocus: the filter's autofocus on mount can
    // land before the window has focus, and then no focus event says so.
    // Focus on the body is focus dropped, which leaves the zone as it was.
    const active = document.activeElement;
    if (active !== document.body) setZone(zoneOf(active));
    // A row removed while focused leaves focus on the body.
    if (refocus.current && document.activeElement === document.body) {
      refocus.current = false;
      ((selected && rowButton(selected)) || filter.current)?.focus();
    }
  });

  useEffect(() => {
    if (selected) rowItem(selected)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const move = (step: 1 | -1) => {
    const next = moveSelection(order.current, selected, step);
    setSelected(next);
    return next;
  };

  return {
    filter: {
      ref: filter,
      onKeyDown: (event) => {
        // Enter and the arrows belong to an input method while it composes.
        if (event.nativeEvent.isComposing) return;
        const command = filterCommand(event);
        if (command === null) return;
        if (command === "list") {
          const target = selected ?? order.current[0];
          // With no rows, Tab goes where it always did.
          if (target === undefined) return;
          event.preventDefault();
          rowButton(target)?.focus();
          return;
        }
        // Also keeps the arrows from moving the caret.
        event.preventDefault();
        if (command === "open") {
          // A click, so the row opens exactly as it does under the pointer.
          if (selected) rowButton(selected)?.click();
          return;
        }
        const next = move(command === "next" ? 1 : -1);
        setAnnouncement((next && view && findRow(view, next)?.label) || "");
      },
    },
    list: {
      ref: list,
      selected,
      select: setSelected,
      acted: () => {
        refocus.current = true;
      },
      onKeyDown: (event) => {
        // An open row menu has keys of its own, and its events bubble here.
        if ((event.target as Element).closest(".pr-menu")) return;
        const command = listCommand(event);
        if (command === null) return;
        event.preventDefault();
        if (command === "filter") {
          filter.current?.focus();
          return;
        }
        const next = move(command === "next" ? 1 : -1);
        if (next) rowButton(next)?.focus();
      },
    },
    around: {
      onFocus: (event) => {
        // Focus moved by hand: nothing is owed a refocus any more.
        refocus.current = false;
        const zone = zoneOf(event.target as Node);
        setZone(zone);
        // Emptied once focus leaves the filter, so coming back to the same
        // row is a change the live region speaks again.
        if (zone !== "filter") setAnnouncement("");
      },
      // A click on nothing focusable drops focus to the body by choice;
      // an action that removed no row must not pull it back later.
      onPointerDown: () => {
        refocus.current = false;
      },
    },
    hints: keyHints(
      zone,
      (selected && view && findRow(view, selected)?.actions) || null,
    ),
    announcement,
  };
}

/* What a row's li takes to join the keyboard model: the key the selection
   knows it by, selection following focus into it, and the row's own keys.
   `run` is the row's action runner. Spread after the menu's handlers: its
   onKeyDown includes theirs. */
function useRowKeys(
  key: string,
  actions: RowAction[],
  run: (id: RowActionId) => void,
  menu: ReturnType<typeof useRowMenu>,
) {
  const keys = useContext(ListKeysContext);
  return {
    selected: keys.selected === key,
    item: {
      "data-row-key": key,
      onFocus: () => keys.select(key),
      onKeyDown: (event: KeyboardEvent) => {
        menu.item.onKeyDown(event);
        // While the menu is open its keys are its own.
        if (menu.at !== null) return;
        const id = rowCommand(event, actions);
        if (id === null) return;
        event.preventDefault();
        keys.acted();
        run(id);
      },
    },
  };
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
  if (days <= 0) return clockTime(then);
  return days === 1 ? "Yesterday" : dayAndMonth(then);
}

const clockTime = (d: Date) => {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

const dayAndMonth = (d: Date) => `${d.getDate()} ${MONTHS[d.getMonth()]}`;

// The exact local date and time, for tooltips: "16 Jul 2026, 14:59".
function formatExact(iso: string): string {
  const then = new Date(iso);
  return `${dayAndMonth(then)} ${then.getFullYear()}, ${clockTime(then)}`;
}

// The deploy status tooltip: the exact times the row abbreviates, one event
// per line. A deploy under way has no time of its own yet.
export function deployTip(mergedAt: string, deploy: DeployView): string {
  const deployed = deploy.completedAt
    ? `${deploy.label} ${formatExact(deploy.completedAt)}`
    : deploy.label;
  return `Merged ${formatExact(mergedAt)}\n${deployed}`;
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

// The author as the avatar's label names them.
const authorLabel = (author: string, bot: boolean) =>
  bot ? `@${author}, bot` : `@${author}`;

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
         row bought little. The aria-label is what puts the handle in the
         accessible name of the row button around it. pr-tip-right because
         the avatar sits at the card's left edge, where the default leftward
         bubble would clip. */
      className="pr-avatar-wrap pr-tip pr-tip-right"
      role="img"
      data-tip={`@${author}`}
      aria-label={authorLabel(author, bot)}
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

// The shared frame around every stroke glyph: shapes are drawn on a 24-unit
// grid and scaled down to a square of `size`. Hidden from assistive tech in
// all uses: the enclosing element carries the label (the section toggle's
// text, a control's or header icon button's aria-label).
export function GlyphSvg({
  strokeWidth,
  size = 13,
  className,
  children,
}: {
  strokeWidth: number;
  size?: number;
  className?: string;
  children: ReactNode;
}) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      width={size}
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

/* The row's state in words: one reason in its tone, then "+N" when further
   states are folded behind it. Which states a row has, and their order, is
   decided by the popover view. Only a folded row gets the tooltip and the
   label, each listing every state; a lone reason is its own label. */
function StateView({ state }: { state: RowState }) {
  const className = `pr-state is-${state.tone}`;
  const reason = <span className="pr-state-reason">{state.reason}</span>;
  if (state.more === null) return <span className={className}>{reason}</span>;
  return (
    <span
      className={`${className} pr-tip`}
      role="img"
      data-tip={state.all.join("\n")}
      aria-label={state.all.join(", ")}
    >
      {reason}
      <span className="pr-state-more">{state.more}</span>
    </span>
  );
}

// What the bare "Confirm" of an armed fix is confirming.
const CONFIRM_HINT = "Confirm: ask Copilot to fix the conflict";

// How long an armed fix waits for its second click before standing down.
const FIX_CONFIRM_MS = 4000;

/* A row's Fix with Copilot action. It posts a comment under the user's name,
   so it takes two clicks from the bar or the menu alike: the first arms it,
   and it reads Confirm until the second, standing down after a few seconds
   or when the row says so. */
function useFix(row: RowView) {
  const [armed, setArmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), FIX_CONFIRM_MS);
    return () => clearTimeout(timer);
  }, [armed]);

  return {
    armed,
    click: () => {
      const next = clickFix(prKey(row.pr), armed);
      setArmed(next.armed);
      setError(null);
      next.request?.then(setError);
    },
    standDown: () => setArmed(false),
    // Shown only beside the offer it explains: once the row stops offering
    // the fix, a note about a failed try has nothing left to say.
    error: row.fix === "offer" ? error : null,
  };
}

/* A row's right-click menu: where it is open, and the handlers that open it.
   `row` goes on the row button, which is where focus returns when the menu
   closes; `item` goes on the li, so a right-click or the context-menu key
   anywhere in the row opens it. A row with no actions has no menu to open. */
function useRowMenu(hasActions: boolean) {
  const [at, setAt] = useState<Point | null>(null);
  const row = useRef<HTMLButtonElement>(null);
  // For the keyboard, which has no pointer to open the menu at: just inside
  // the row, clear of the avatar.
  const openAtRow = () => {
    const box = row.current?.getBoundingClientRect();
    if (box) setAt({ x: box.left + 48, y: box.top + box.height / 2 });
  };
  return {
    at,
    row,
    close: (refocus: boolean) => {
      setAt(null);
      if (refocus) row.current?.focus();
    },
    item: {
      onContextMenu: (event: MouseEvent) => {
        // Always, so the webview's own menu never shows over a row.
        event.preventDefault();
        if (!hasActions) return;
        if ((event.target as Element).closest(".pr-menu")) return;
        // A context menu asked for by key arrives without a pointer position.
        if (event.clientX || event.clientY) {
          setAt({ x: event.clientX, y: event.clientY });
        } else {
          openAtRow();
        }
      },
      onKeyDown: (event: KeyboardEvent) => {
        if (!hasActions || at !== null || !opensMenu(event)) return;
        event.preventDefault();
        openAtRow();
      },
    },
  };
}

/* An open row menu, mounted on the body rather than in its row: nothing
   there clips it or scrolls it, and it reaches past the row it belongs to.
   Its events still bubble to the row through React's tree. */
function MenuLayer(props: Parameters<typeof RowMenu>[0]) {
  return createPortal(<RowMenu {...props} />, document.body);
}

// The li of a row: lit while its menu is open, so the menu's row is plain
// even with the pointer on the menu, and tinted while it is the selection.
const rowClass = (menuOpen: boolean, selected: boolean) =>
  "pr-row-split" +
  (menuOpen ? " has-menu" : "") +
  (selected ? " is-selected" : "");

/* The row's actions as worded buttons. Siblings of the row button rather than
   children (nested buttons are invalid HTML), floated over the end of the
   row's metadata line and shown only while the row is hovered or holds
   focus. `confirm` is the action waiting for its confirming click, under the
   words it shows meanwhile, and `status` stands where an action would while
   it cannot be asked for. A row with neither has no bar. */
function ActionBar({
  actions,
  confirm = null,
  status = null,
  onPick,
}: {
  actions: RowAction[];
  confirm?: RowAction | null;
  status?: string | null;
  onPick: (id: RowActionId) => void;
}) {
  if (actions.length === 0 && !status) return null;
  return (
    <span className="pr-actions" role="group" aria-label="Actions">
      {status && (
        <span className="pr-action-status" role="status">
          {status}
        </span>
      )}
      {actions.map((action) => {
        const { id, label, hint } =
          action.id === confirm?.id ? confirm : action;
        return (
          <button
            key={id}
            type="button"
            className={id === confirm?.id ? "pr-action is-armed" : "pr-action"}
            title={hint ?? undefined}
            // The hint is what the label leaves unsaid, so it names the
            // button in full; it starts with the label, as a spoken name
            // must.
            aria-label={hint ?? undefined}
            onClick={() => onPick(id)}
          >
            {id === confirm?.id ? (
              /* The armed button keeps the width of the label it replaced.
                 The bar is right-aligned, so a narrower button would slide
                 its neighbour under the pointer, and the confirming click
                 would land on that instead. */
              <>
                <span className="pr-action-width" aria-hidden="true">
                  {action.label}
                </span>
                <span className="pr-action-over">{label}</span>
              </>
            ) : (
              label
            )}
          </button>
        );
      })}
    </span>
  );
}

// showRepo is false inside a repo group, where the header already names it.
function PrRow({ row, showRepo = true }: { row: RowView; showRepo?: boolean }) {
  const { pr, draft, state, botAuthor, botNote, waiting } = row;
  const unread = pr.unread_count > 0;
  const key = prKey(pr);
  const fix = useFix(row);
  const menu = useRowMenu(row.actions.length > 0);
  // The armed fix under the words it takes until its second click, in the
  // bar and the menu alike.
  const confirm: RowAction | null = fix.armed
    ? { id: "fix", label: "Confirm", hint: CONFIRM_HINT }
    : null;
  const open = () => {
    openUrl(pr.url).catch(() => {});
    // The backend clears the badge and pushes a fresh snapshot.
    markRead(key).catch(() => {});
  };
  const run = (id: RowActionId) => {
    switch (id) {
      case "snooze":
        snooze(key).catch(() => {});
        break;
      case "wake":
        wake(key).catch(() => {});
        break;
      case "fix":
        fix.click();
        break;
    }
  };
  const rowKeys = useRowKeys(key, row.actions, run, menu);
  return (
    <li
      className={rowClass(menu.at !== null, rowKeys.selected)}
      // Not while the menu is open: the pointer may be nowhere near a menu
      // opened by key, and the armed item must wait there for its confirm.
      onPointerLeave={menu.at === null ? fix.standDown : undefined}
      {...menu.item}
      {...rowKeys.item}
    >
      <button
        ref={menu.row}
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
          <span
            className={waiting ? "pr-title-row is-waiting" : "pr-title-row"}
          >
            <span className="pr-title">{pr.title}</span>
            <span className="pr-updated">
              {waiting ?? formatUpdated(pr.updated_at)}
            </span>
          </span>
          <span className="pr-origin">
            {/* Number first, in its own non-shrinking span, so "PR 510" is
                findable at a fixed x-position on every row: the repo and
                author give way to truncation, the number never does. */}
            <span className="pr-number">#{pr.number}</span>
            {showRepo && <span className="pr-repo">{pr.repo}</span>}
            {/* Plain metadata, not a badge: bots spoke, nobody is waiting. */}
            {botNote && <span className="pr-bot-note">{botNote}</span>}
            {/* Why the Copilot request did not go out. title carries the
                full message; the line clips it. */}
            {fix.error && (
              <span className="pr-fix-error" role="alert" title={fix.error}>
                {fix.error}
              </span>
            )}
            {/* A neutral state pill, never a signal. */}
            {draft && <span className="pr-draft">Draft</span>}
            {/* At the row's right edge, out of the metadata's way. */}
            {state && <StateView state={state} />}
          </span>
        </span>
      </button>
      <span className="pr-row-end">
        <ActionBar
          actions={row.actions}
          confirm={confirm}
          status={row.fix === "posting" ? "Asking" : null}
          onPick={run}
        />
      </span>
      {menu.at && (
        <MenuLayer
          at={menu.at}
          items={row.actions.map((action) =>
            action.id === confirm?.id ? confirm : action,
          )}
          onPick={(id) => {
            run(id);
            // The pick that arms the fix leaves the menu up for its confirm.
            if (id !== "fix" || fix.armed) menu.close(true);
          }}
          onClose={menu.close}
        />
      )}
    </li>
  );
}

/* The snoozed rows of a section behind one folded line at its foot. Whether
   the line is open is not persisted: it starts folded whenever the section
   is rendered afresh, since snoozed rows were put out of the way on purpose. */
function SnoozedGroup({ group }: { group: SnoozedGroupView }) {
  const [opened, setOpened] = useState(false);
  const open = opened || group.forcedOpen;
  return (
    <>
      <li className="pr-snoozed-line">
        <button
          type="button"
          className="pr-snoozed-toggle"
          aria-expanded={open}
          aria-disabled={group.forcedOpen}
          onClick={group.forcedOpen ? undefined : () => setOpened(!opened)}
        >
          <GlyphSvg strokeWidth={2.5} size={10} className="pr-section-chevron">
            <path d="M9 5l7 7-7 7" />
          </GlyphSvg>
          {group.label}
        </button>
      </li>
      {open &&
        group.rows.map((row) => <PrRow key={prKey(row.pr)} row={row} />)}
    </>
  );
}

// A rocket, for a merged row's deploy status. Inherits the status's colour,
// so a failed deploy's is red.
function DeployGlyph() {
  return (
    <GlyphSvg strokeWidth={2} size={12}>
      <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
      <path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
      <path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0" />
      <path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5" />
    </GlyphSvg>
  );
}

/* The deploy status is a control of its own (it opens the workflow run, the
   row opens the PR), so like the action bar it is a sibling of the row
   button, not nested in it, which would be invalid HTML. No mark_read here:
   merged PRs carry no unread state. */
function MergedRow({ row }: { row: MergedRowView }) {
  const { pr, deploy } = row;
  const menu = useRowMenu(row.actions.length > 0);
  // A deploy that landed shows when, under the merge time; any other says
  // where it stands. The glyph is what says "deploy" in either.
  const statusText =
    deploy &&
    (deploy.showsTime && deploy.completedAt
      ? formatUpdated(deploy.completedAt)
      : deploy.brief);
  const status = deploy && (
    <>
      <DeployGlyph />
      {statusText}
    </>
  );
  const open = () => openUrl(pr.url).catch(() => {});
  const run = (id: RowActionId) => {
    if (id === "dismiss") dismissMerged(prKey(pr)).catch(() => {});
  };
  const rowKeys = useRowKeys(prKey(pr), row.actions, run, menu);
  return (
    <li
      className={rowClass(menu.at !== null, rowKeys.selected)}
      {...menu.item}
      {...rowKeys.item}
    >
      <button
        ref={menu.row}
        type="button"
        className="pr-row"
        onClick={open}
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
            {/* Holds the status's place in the line so the repo truncates
                before it; the real control floats over this spot. */}
            {status && (
              <span className="pr-deploy-space" aria-hidden="true">
                {status}
              </span>
            )}
          </span>
        </span>
      </button>
      <span className="pr-row-end">
        <ActionBar actions={row.actions} onPick={run} />
        {deploy && (
          <button
            type="button"
            className={deploy.failed ? "pr-deploy is-failed" : "pr-deploy"}
            title={deployTip(pr.merged_at, deploy)}
            // The glyph is unseen by assistive tech, and a bare time would
            // not say what it is the time of.
            aria-label={
              deploy.showsTime && deploy.completedAt
                ? `${deploy.label} ${statusText}`
                : deploy.label
            }
            onClick={() => openUrl(deploy.url).catch(() => {})}
          >
            {status}
          </button>
        )}
      </span>
      {menu.at && (
        <MenuLayer
          at={menu.at}
          items={row.actions}
          onPick={(id) => {
            run(id);
            menu.close(true);
          }}
          onClose={menu.close}
        />
      )}
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

/* The heading of Todo: the same label and count as a SectionHeader,
   with no button around them, since the section is always open. */
function PinnedHeader({ label, count }: { label: string; count: number }) {
  return (
    <div className="pr-section-header">
      <h2 className="pr-section-heading pr-section-pinned">
        <span className="pr-section-label">{label}</span>
        <span className="pr-section-count" aria-hidden="true">
          {count}
        </span>
        <span className="visually-hidden">
          {`${count} pull request${count === 1 ? "" : "s"}`}
        </span>
      </h2>
    </div>
  );
}

function SectionBodyView({ section }: { section: SectionView }) {
  const { body } = section;
  switch (body.kind) {
    case "empty":
      /* Inside the slab, not bare text, so an empty section keeps the shape
         rows will appear in. A grey dash, not a sentence: empty is the
         ordinary state of a section and has nothing to announce. */
      return (
        <ul>
          <li className="pr-empty">
            <span role="img" aria-label="No pull requests">
              <GlyphSvg strokeWidth={2}>
                <path d="M7 12h10" />
              </GlyphSvg>
            </span>
          </li>
        </ul>
      );
    case "rows":
      return (
        <ul>
          {body.rows.map((row) => (
            <PrRow key={prKey(row.pr)} row={row} />
          ))}
          {body.snoozed && <SnoozedGroup group={body.snoozed} />}
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
          {body.rows.map((row) => (
            <MergedRow key={prKey(row.pr)} row={row} />
          ))}
        </ul>
      );
  }
}

// Renders the sections it is given: which exist, what they hold and whether
// they are open is the popover view's call. `keys` is useListKeys' `list`;
// without it the list has no selection and no keys of its own.
function PrList({
  view,
  onToggle,
  keys = NO_LIST_KEYS,
}: {
  view: ListView;
  onToggle: (key: SectionKey) => void;
  keys?: ListKeys;
}) {
  return (
    <main className="pr-list" ref={keys.ref} onKeyDown={keys.onKeyDown}>
      <ListKeysContext.Provider value={keys}>
        {view.sections.map((section) => {
          const { key } = section;
          return (
            <section key={key} className="pr-section">
              {key === YOUR_TURN_KEY ? (
                <PinnedHeader label={section.label} count={section.count} />
              ) : (
                <SectionHeader
                  label={section.label}
                  count={section.count}
                  expanded={section.expanded}
                  onToggle={() => onToggle(key)}
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
              )}
              {section.expanded && <SectionBodyView section={section} />}
            </section>
          );
        })}
      </ListKeysContext.Provider>
    </main>
  );
}

export default PrList;
