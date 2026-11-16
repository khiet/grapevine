import {
  prKey,
  SNAPSHOT_EVENT,
  type Adapter,
  type Appearance,
  type PrAction,
  type PullRequest,
  type Snapshot,
} from "./index";

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
  let appearance: Appearance = "system";
  const subscribers = new Set<(snapshot: Snapshot) => void>();

  const publish = (next: Snapshot) => {
    snapshot = next;
    for (const subscriber of subscribers) subscriber(snapshot);
    onPublish(snapshot);
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_prs: () => snapshot,
    get_appearance: () => appearance,
    set_appearance: (args) => (appearance = args.appearance as Appearance),
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
    // Refuses what the backend would, then leaves the PR as GitHub would
    // have it after the write, as far as the fake can tell: it has no sync
    // to learn the rest from.
    act_on_pr: ({ key, action, head }) => {
      const pr = snapshot.prs.find((candidate) => prKey(candidate) === key);
      if (!pr) throw "That PR is no longer in the list.";
      if (snapshot.snoozed.includes(key as string)) {
        throw "That PR is snoozed. Wake it first.";
      }
      if (pr.head_oid !== head) {
        throw "That PR has new commits. Check it, then try again.";
      }
      if (!pr.actions.includes(action as PrAction)) {
        throw "That is no longer on offer for this PR.";
      }
      if (action === "merge") {
        publish({
          ...snapshot,
          prs: snapshot.prs.filter((candidate) => candidate !== pr),
          merged: [
            {
              number: pr.number,
              title: pr.title,
              url: pr.url,
              repo: pr.repo,
              author: pr.author,
              avatar_url: pr.avatar_url,
              owner_avatar_url: pr.owner_avatar_url,
              // The fake has no clock; the PR's last update stands in.
              merged_at: pr.updated_at,
              deploy: null,
            },
            ...snapshot.merged,
          ],
        });
        return;
      }
      // A merge under way waits on GitHub, not on the viewer.
      const underWay = {
        ready_to_merge: false,
        turn: "theirs",
        turn_since: null,
      } as const;
      const after: Record<Exclude<PrAction, "merge">, Partial<PullRequest>> = {
        queue: { ...underWay, merge_queue_position: 1, actions: [] },
        update_branch: {
          blocked_reasons: pr.blocked_reasons.filter((r) => r !== "behind"),
          actions: pr.actions.filter((a) => a !== "update_branch"),
        },
        enable_auto_merge: {
          ...underWay,
          auto_merge: true,
          actions: ["disable_auto_merge"],
        },
        disable_auto_merge: { auto_merge: false, actions: ["enable_auto_merge"] },
      };
      const changes = after[action as Exclude<PrAction, "merge">];
      publish({
        ...snapshot,
        prs: snapshot.prs.map((candidate) =>
          candidate === pr ? { ...pr, ...changes } : candidate,
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
