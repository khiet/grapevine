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
  const listeners = new Map<string, Set<(payload: never) => void>>();

  const publish = (next: Snapshot) => {
    snapshot = next;
    for (const handler of listeners.get(SNAPSHOT_EVENT) ?? []) {
      (handler as (payload: Snapshot) => void)(snapshot);
    }
    onPublish(snapshot);
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_prs: () => snapshot,
    mark_read: ({ key }) =>
      publish({
        ...snapshot,
        prs: snapshot.prs.map((pr) =>
          prKey(pr) === key ? { ...pr, unread_count: 0 } : pr,
        ),
      }),
    mark_all_read: () =>
      publish({
        ...snapshot,
        prs: snapshot.prs.map((pr) => ({ ...pr, unread_count: 0 })),
      }),
    dismiss_merged: ({ key }) =>
      publish({
        ...snapshot,
        merged: snapshot.merged.filter((pr) => prKey(pr) !== key),
      }),
    clear_merged: () => publish({ ...snapshot, merged: [] }),
  };

  const adapter: Adapter = {
    async invoke<T>(command: string, args: Record<string, unknown> = {}) {
      const handler = commands[command];
      if (!handler) throw new Error(`fake backend: unhandled command ${command}`);
      return handler(args) as T;
    },
    async listen<T>(event: string, handler: (payload: T) => void) {
      const handlers = listeners.get(event) ?? new Set();
      listeners.set(event, handlers);
      handlers.add(handler as (payload: never) => void);
      return () => {
        handlers.delete(handler as (payload: never) => void);
      };
    },
    async openUrl() {},
    async appVersion() {
      return "0.0.0";
    },
  };

  return { adapter, publish };
}
