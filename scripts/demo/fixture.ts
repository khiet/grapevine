import type { MergedPr, PullRequest, Snapshot } from "../../src/backend";

// Every PR here is fictional; the people and repo owners are real GitHub
// accounts. The capture clock in demo.mjs is set just after the newest
// timestamp below.
const ME = "khiet";
const at = (time: string) => `2026-09-${time}+01:00`;

/** The Snapshot the demo popover renders. `avatars` maps each author and repo
 * owner login to an image URL. */
export function demoSnapshot(avatars: Record<string, string>): Snapshot {
  const pr = (
    repo: string,
    number: number,
    title: string,
    author: string,
    updated: string,
    extra: Partial<PullRequest> = {},
  ): PullRequest => ({
    number,
    title,
    url: `https://github.com/${repo}/pull/${number}`,
    repo,
    author,
    avatar_url: avatars[author],
    owner_avatar_url: avatars[repo.split("/")[0]],
    created_at: at("20T09:00:00"),
    updated_at: at(updated),
    section: "participated",
    blocked_reasons: [],
    is_draft: false,
    review_requested: false,
    awaiting_review: false,
    approved: false,
    merge_queue_position: null,
    unread_count: 0,
    ...extra,
  });
  const merged = (
    repo: string,
    number: number,
    title: string,
    author: string,
    mergedAt: string,
  ): MergedPr => ({
    number,
    title,
    url: `https://github.com/${repo}/pull/${number}`,
    repo,
    author,
    avatar_url: avatars[author],
    owner_avatar_url: avatars[repo.split("/")[0]],
    merged_at: at(mergedAt),
  });
  const mine = { section: "mine" } as const;
  const all = { section: "all" } as const;
  return {
    has_synced: true,
    last_sync_at: Date.parse(at("30T15:41:00")),
    sync_error: null,
    prs: [
      pr("teampave/checkout-api", 1284, "Retry failed webhook deliveries with exponential backoff", "doutatsu", "30T15:31:00", { unread_count: 3, review_requested: true }),
      pr("teampave/checkout-api", 1291, "Add idempotency keys to POST /payments", ME, "30T15:12:00", { ...mine, unread_count: 1, awaiting_review: true }),
      pr("teampave/web-app", 2317, "Add saved payment methods to the checkout form", "MarcusAl", "30T14:58:00", { unread_count: 1, blocked_reasons: ["threads"] }),
      pr("fuseinsight/ingest", 412, "Stream podcast RSS imports instead of buffering them", "doutatsu", "30T12:10:00", { approved: true }),
      pr("teampave/web-app", 2320, "Upgrade to React 19 and drop legacy context", ME, "30T11:40:00", { ...mine, approved: true, merge_queue_position: 2 }),
      pr("khiet/design-tokens", 88, "Rename the spacing scale to t-shirt sizes", "MarcusAl", "29T17:20:00", { review_requested: true }),
      pr("fuseinsight/ingest", 415, "Parse chapter markers from ID3 tags", ME, "29T16:05:00", { ...mine, blocked_reasons: ["ci"] }),
      pr("teampave/checkout-api", 1279, "Fix rounding in multi-currency refunds", "doutatsu", "28T10:45:00", { unread_count: 2, blocked_reasons: ["conflict", "behind"] }),
      pr("teampave/web-app", 2322, "Spike: dark mode for the receipt page", ME, "26T14:30:00", { ...mine, is_draft: true }),
      pr("teampave/web-app", 2315, "Lazy-load the order history table", "MarcusAl", "30T10:20:00", all),
      pr("teampave/checkout-api", 1270, "Bump stripe-node to 17.4", "doutatsu", "29T08:00:00", all),
      pr("fuseinsight/ingest", 409, "Move transcoding jobs to the shared queue", "MarcusAl", "27T13:15:00", all),
    ],
    merged: [
      merged("teampave/web-app", 2309, "Show delivery ETA on the order confirmation", ME, "30T13:05:00"),
      merged("khiet/design-tokens", 85, "Add focus ring tokens", "MarcusAl", "29T15:50:00"),
    ],
  };
}
