import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { checkForUpdates, updateStatusLabel, useUpdateState } from "./updater";

interface TokenStatus {
  has_token: boolean;
  login: string | null;
}

interface SavedToken {
  login: string;
  scope_warning: string | null;
}

// The query parameters pre-tick the scope and fill the description. This is
// undocumented GitHub behaviour, but degrades to a plain token page if it ever
// stops working, leaving the note copy to carry the guidance alone.
const CREATE_TOKEN_URL =
  "https://github.com/settings/tokens/new?scopes=repo&description=Grapevine";

// A curated menu, not the real bounds: the backend accepts anything in 30s..1h.
// A hand-edited settings.json value outside the list is appended as its own
// option so the select never lies about the stored state.
const POLL_PRESETS = [60, 120, 180, 300, 600, 900, 1800, 3600];

export function pollLabel(secs: number): string {
  if (secs === 3600) return "1 hour";
  if (secs < 60 || secs % 60 !== 0) return `${secs} seconds`;
  const minutes = secs / 60;
  return minutes === 1 ? "1 minute" : `${minutes} minutes`;
}

export interface RepoEntry {
  fullName: string;
  name: string;
  watched: boolean;
}

export interface RepoGroup {
  owner: string;
  repos: RepoEntry[];
}

// The browse list is the union of the fetched affiliated repos and the
// watched list: a watched repo outside the fetch (external OSS, or beyond
// the backend's page cap) must stay visible or it could never be unchecked.
// Matching is case-insensitive with the fetched (canonical) casing winning.
// Groups sort by owner, repos by name, both case-insensitively.
export function groupRepos(available: string[], watched: string[]): RepoGroup[] {
  const watchedKeys = new Set(watched.map((repo) => repo.toLowerCase()));
  const availableKeys = new Set(available.map((repo) => repo.toLowerCase()));
  const names = [
    ...available,
    ...watched.filter((repo) => !availableKeys.has(repo.toLowerCase())),
  ];
  const groups = new Map<string, RepoGroup>();
  for (const fullName of names) {
    const slash = fullName.indexOf("/");
    const owner = slash === -1 ? fullName : fullName.slice(0, slash);
    const name = slash === -1 ? fullName : fullName.slice(slash + 1);
    const key = owner.toLowerCase();
    let group = groups.get(key);
    if (!group) {
      group = { owner, repos: [] };
      groups.set(key, group);
    }
    group.repos.push({
      fullName,
      name,
      watched: watchedKeys.has(fullName.toLowerCase()),
    });
  }
  const compare = (a: string, b: string) =>
    a.localeCompare(b, undefined, { sensitivity: "base" });
  const result = [...groups.values()];
  for (const group of result) {
    group.repos.sort((a, b) => compare(a.name, b.name));
  }
  result.sort((a, b) => compare(a.owner, b.owner));
  return result;
}

// Case-insensitive substring match against the full "owner/name" string, so
// one needle can hit the owner, the repo name, or span the slash. Groups
// left with no matches disappear.
export function filterGroups(groups: RepoGroup[], filter: string): RepoGroup[] {
  const needle = filter.trim().toLowerCase();
  if (needle === "") return groups;
  return groups
    .map((group) => ({
      owner: group.owner,
      repos: group.repos.filter((repo) =>
        repo.fullName.toLowerCase().includes(needle),
      ),
    }))
    .filter((group) => group.repos.length > 0);
}

function SettingsView() {
  const [tokenStatus, setTokenStatus] = useState<TokenStatus>({
    has_token: false,
    login: null,
  });
  const [tokenInput, setTokenInput] = useState("");
  const [tokenError, setTokenError] = useState("");
  // Non-blocking: the token is stored even when this is set. Feedback on the
  // save action only, so it does not reappear after a restart.
  const [tokenWarning, setTokenWarning] = useState("");
  const [tokenBusy, setTokenBusy] = useState(false);

  const [repos, setRepos] = useState<string[]>([]);
  const [repoInput, setRepoInput] = useState("");
  const [repoError, setRepoError] = useState("");
  const [repoBusy, setRepoBusy] = useState(false);

  // null until the affiliated-repo fetch resolves; [] is a real empty result.
  const [available, setAvailable] = useState<string[] | null>(null);
  const [browseError, setBrowseError] = useState("");
  const [repoFilter, setRepoFilter] = useState("");
  // Lowercased names of repos with an in-flight toggle: their checkboxes are
  // disabled so a double click cannot fire add and remove concurrently.
  const [toggling, setToggling] = useState<Set<string>>(new Set());
  // add_repo/remove_repo each return the authoritative repo list; when two
  // toggles overlap, only the response to the latest request may win, or a
  // stale list would resurrect the earlier toggle's state.
  const repoSeq = useRef(0);

  // null until the stored value arrives, so the select never flashes a
  // default the user did not pick.
  const [pollSecs, setPollSecs] = useState<number | null>(null);
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  const [generalError, setGeneralError] = useState("");

  const [appVersion, setAppVersion] = useState("");
  const updateState = useUpdateState();

  useEffect(() => {
    invoke<TokenStatus>("token_status").then(setTokenStatus).catch(() => {});
    invoke<string[]>("list_repos").then(setRepos).catch(() => {});
    invoke<number>("get_poll_interval").then(setPollSecs).catch(() => {});
    invoke<boolean>("get_launch_at_login").then(setLaunchAtLogin).catch(() => {});
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  const loadAvailable = useCallback(async () => {
    setBrowseError("");
    try {
      setAvailable(await invoke<string[]>("list_affiliated_repos"));
    } catch (error) {
      setBrowseError(String(error));
    }
  }, []);

  // Gated on has_token rather than run on mount: token_status resolves
  // asynchronously, and this also re-fetches the moment a token is saved.
  useEffect(() => {
    if (tokenStatus.has_token) loadAvailable();
  }, [tokenStatus.has_token, loadAvailable]);

  async function changePollInterval(secs: number) {
    setGeneralError("");
    try {
      setPollSecs(await invoke<number>("set_poll_interval", { secs }));
    } catch (error) {
      setGeneralError(String(error));
    }
  }

  async function toggleLaunchAtLogin(enabled: boolean) {
    setGeneralError("");
    setLaunchAtLogin(enabled);
    try {
      await invoke("set_launch_at_login", { enabled });
    } catch (error) {
      setLaunchAtLogin(!enabled);
      setGeneralError(String(error));
    }
  }

  async function saveToken(event: FormEvent) {
    event.preventDefault();
    setTokenBusy(true);
    setTokenError("");
    setTokenWarning("");
    try {
      const saved = await invoke<SavedToken>("save_token", { token: tokenInput });
      setTokenStatus({ has_token: true, login: saved.login });
      setTokenWarning(saved.scope_warning ?? "");
      setTokenInput("");
    } catch (error) {
      setTokenError(String(error));
    } finally {
      setTokenBusy(false);
    }
  }

  async function removeToken() {
    setTokenError("");
    setTokenWarning("");
    try {
      await invoke("clear_token");
      setTokenStatus({ has_token: false, login: null });
    } catch (error) {
      setTokenError(String(error));
    }
  }

  async function addRepo(event: FormEvent) {
    event.preventDefault();
    setRepoBusy(true);
    setRepoError("");
    const seq = ++repoSeq.current;
    try {
      const next = await invoke<string[]>("add_repo", { name: repoInput });
      if (seq === repoSeq.current) setRepos(next);
      setRepoInput("");
    } catch (error) {
      setRepoError(String(error));
    } finally {
      setRepoBusy(false);
    }
  }

  async function toggleRepo(fullName: string, watched: boolean) {
    const key = fullName.toLowerCase();
    if (toggling.has(key)) return;
    setToggling((prev) => new Set(prev).add(key));
    setRepoError("");
    const seq = ++repoSeq.current;
    try {
      const next = await invoke<string[]>(watched ? "remove_repo" : "add_repo", {
        name: fullName,
      });
      if (seq === repoSeq.current) setRepos(next);
    } catch (error) {
      setRepoError(String(error));
    } finally {
      setToggling((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  }

  // A failed or skipped fetch leaves `available` empty, so the union
  // degrades to the watched repos rendered as checked rows.
  const browseGroups = filterGroups(
    groupRepos(available ?? [], repos),
    repoFilter,
  );

  return (
    <main className="settings">
      <section className="settings-section">
        <h2 className="settings-label">GitHub token</h2>
        <div className="settings-card">
          {tokenStatus.has_token && (
            <div className="settings-row">
              <span className="settings-ok">
                Signed in as <strong>{tokenStatus.login ?? "unknown"}</strong>
              </span>
              <button type="button" className="settings-link" onClick={removeToken}>
                Remove
              </button>
            </div>
          )}
          <form className="settings-row settings-input-row" onSubmit={saveToken}>
            <input
              type="password"
              value={tokenInput}
              onChange={(event) => setTokenInput(event.target.value)}
              placeholder={
                tokenStatus.has_token
                  ? "Replace token"
                  : "Personal access token (classic)"
              }
              autoComplete="off"
              spellCheck={false}
            />
            <button type="submit" disabled={tokenBusy || tokenInput.trim() === ""}>
              {tokenBusy ? "Checking…" : "Save"}
            </button>
          </form>
          {/* Always visible, not just when no token is saved: tokens expire
              every 30 days by default and get re-minted. */}
          <p className="settings-note">
            Need a token?{" "}
            <button
              type="button"
              className="settings-link"
              onClick={() => openUrl(CREATE_TOKEN_URL).catch(() => {})}
            >
              Create one on GitHub
            </button>
            . <strong>repo</strong> grants read and write access to all your
            private repositories; Grapevine only ever reads. If you watch
            public repositories only, <strong>public_repo</strong> is enough.
          </p>
        </div>
        {tokenError && <p className="settings-error">{tokenError}</p>}
        {tokenWarning && <p className="settings-warning">{tokenWarning}</p>}
      </section>

      <section className="settings-section">
        <h2 className="settings-label">Watched repositories</h2>
        <div className="settings-card">
          <div className="settings-row settings-input-row">
            <input
              type="search"
              value={repoFilter}
              onChange={(event) => setRepoFilter(event.target.value)}
              placeholder="Filter repositories"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          {!tokenStatus.has_token ? (
            <p className="settings-note">
              Save a GitHub token above to browse your repositories.
            </p>
          ) : available === null && !browseError ? (
            <p className="settings-row settings-empty">Loading repositories…</p>
          ) : null}
          {browseGroups.length > 0 ? (
            <div className="repo-browser">
              {browseGroups.map((group) => (
                <div key={group.owner.toLowerCase()}>
                  <div className="repo-group-label">{group.owner}</div>
                  {group.repos.map((repo) => (
                    <label
                      key={repo.fullName.toLowerCase()}
                      className="settings-row repo-check-row"
                    >
                      <input
                        type="checkbox"
                        checked={repo.watched}
                        disabled={toggling.has(repo.fullName.toLowerCase())}
                        onChange={() => toggleRepo(repo.fullName, repo.watched)}
                      />
                      <span className="repo-name">{repo.name}</span>
                    </label>
                  ))}
                </div>
              ))}
            </div>
          ) : repoFilter.trim() !== "" ? (
            // Only a filter can empty a non-empty list; the other empty
            // states (no token, loading) already have their own rows.
            <p className="settings-row settings-empty">No repositories match.</p>
          ) : null}
          {browseError && (
            <p className="settings-row settings-warning">
              <span>{browseError}</span>
              <button
                type="button"
                className="settings-link"
                onClick={loadAvailable}
              >
                Retry
              </button>
            </p>
          )}
          {/* The browse list only offers owned and org repos, so this is the
              path for everything else: external OSS, collaborator repos. */}
          <form className="settings-manual" onSubmit={addRepo}>
            <p className="manual-note">Not listed? Add any repository by name.</p>
            <div className="settings-input-row">
              <input
                value={repoInput}
                onChange={(event) => setRepoInput(event.target.value)}
                placeholder="owner/repo"
                autoComplete="off"
                spellCheck={false}
              />
              <button
                type="submit"
                disabled={repoBusy || repoInput.trim() === ""}
              >
                {repoBusy ? "Checking…" : "Add"}
              </button>
            </div>
          </form>
        </div>
        {repoError && <p className="settings-error">{repoError}</p>}
      </section>

      <section className="settings-section">
        <h2 className="settings-label">General</h2>
        <div className="settings-card">
          <label className="settings-row settings-field-row">
            <span>Refresh pull requests every</span>
            <select
              value={pollSecs ?? ""}
              disabled={pollSecs === null}
              onChange={(event) => changePollInterval(Number(event.target.value))}
            >
              {pollSecs === null && <option value="" />}
              {(pollSecs === null || POLL_PRESETS.includes(pollSecs)
                ? POLL_PRESETS
                : [...POLL_PRESETS, pollSecs].sort((a, b) => a - b)
              ).map((secs) => (
                <option key={secs} value={secs}>
                  {pollLabel(secs)}
                </option>
              ))}
            </select>
          </label>
          <label className="settings-row settings-field-row">
            <span>Launch at login</span>
            <input
              type="checkbox"
              className="settings-switch"
              checked={launchAtLogin}
              onChange={(event) => toggleLaunchAtLogin(event.target.checked)}
            />
          </label>
        </div>
        {generalError && <p className="settings-error">{generalError}</p>}
      </section>

      <section className="settings-section">
        <h2 className="settings-label">Updates</h2>
        <div className="settings-card">
          <div className="settings-row settings-field-row">
            <span>Grapevine {appVersion && `v${appVersion}`}</span>
            <button
              type="button"
              disabled={
                updateState.phase === "checking" ||
                updateState.phase === "downloading"
              }
              onClick={() => checkForUpdates(true)}
            >
              Check for updates
            </button>
          </div>
        </div>
        {updateStatusLabel(updateState) && (
          <p className="settings-note">{updateStatusLabel(updateState)}</p>
        )}
      </section>
    </main>
  );
}

export default SettingsView;
