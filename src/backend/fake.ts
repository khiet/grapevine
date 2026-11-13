import { prKey, SNAPSHOT_EVENT, type Adapter, type Snapshot } from "./index";

export interface FakeBackend {
  adapter: Adapter;
  /** Replaces the Snapshot and pushes it to subscribers, as a sync would. */
  publish(next: Snapshot): void;
}

/** An in-memory backend for the demo and for tests. It holds one Snapshot and
 * implements the commands that read or change it; any other command rejects,
 * so a caller that strays outside the fake fails loudly. */
export function createFakeBackend(
  initial: Snapshot,
  // Runs after every push, with the Snapshot just pushed.
  onPublish: (snapshot: Snapshot) => void = () => {},
): FakeBackend {
  let snapshot = initial;
  const subscribers = new Set<(snapshot: Snapshot) => void>();

  const publish = (next: Snapshot) => {
    snapshot = next;
    for (const subscriber of subscribers) subscriber(snapshot);
    onPublish(snapshot);
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_prs: () => snapshot,
    // Claims the sync as the backend does and leaves it running: the fake
    // has no GitHub to ask, so whoever drives it publishes the result.
    sync_now: () => {
      if (!snapshot.syncing) publish({ ...snapshot, syncing: true });
    },
    mark_read: ({ key }) =>
      publish({
        ...snapshot,
        prs: snapshot.prs.map((pr) =>
          prKey(pr) === key
            ? { ...pr, unread_count: 0, bot_unread_count: 0 }
            : pr,
        ),
      }),
    mark_all_read: () =>
      publish({
        ...snapshot,
        prs: snapshot.prs.map((pr) => ({
          ...pr,
          unread_count: 0,
          bot_unread_count: 0,
        })),
      }),
    dismiss_merged: ({ key }) =>
      publish({
        ...snapshot,
        merged: snapshot.merged.filter((pr) => prKey(pr) !== key),
      }),
    clear_merged: () => publish({ ...snapshot, merged: [] }),
    // No expiry and no waking on news: the fake never syncs.
    snooze: ({ key }) =>
      publish({
        ...snapshot,
        snoozed: [...snapshot.snoozed.filter((k) => k !== key), key as string],
      }),
    wake: ({ key }) =>
      publish({
        ...snapshot,
        snoozed: snapshot.snoozed.filter((k) => k !== key),
      }),
    // Refuses what the backend would, and hands a fixable PR to Copilot.
    // There it stays: the fake has no Copilot to clear the conflict and no
    // clock to run the request out.
    fix_conflict: ({ key }) => {
      const pr = snapshot.prs.find((candidate) => prKey(candidate) === key);
      if (pr?.fix_requested) throw "Copilot has already been asked to fix this.";
      if (!pr?.can_fix_conflict) throw "Nothing here for Copilot to fix.";
      const waitsOnCopilot = !pr.review_requested;
      publish({
        ...snapshot,
        prs: snapshot.prs.map((candidate) =>
          candidate === pr
            ? {
                ...pr,
                fix_requested: true,
                turn: waitsOnCopilot ? "theirs" : pr.turn,
                turn_since: waitsOnCopilot ? null : pr.turn_since,
              }
            : candidate,
        ),
      });
    },
  };

  const adapter: Adapter = {
    async invoke<T>(command: string, args: Record<string, unknown> = {}) {
      const handler = commands[command];
      if (!handler) throw new Error(`fake backend: unhandled command ${command}`);
      return handler(args) as T;
    },
    async listen<T>(event: string, handler: (payload: T) => void) {
      // The Snapshot push is the only event this backend emits.
      if (event !== SNAPSHOT_EVENT) return () => {};
      const subscriber = handler as (snapshot: Snapshot) => void;
      subscribers.add(subscriber);
      return () => {
        subscribers.delete(subscriber);
      };
    },
    async openUrl() {},
    async appVersion() {
      return "0.0.0";
    },
  };

  return { adapter, publish };
}
