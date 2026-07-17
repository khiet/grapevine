import { FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

interface TokenStatus {
  has_token: boolean;
  login: string | null;
}

interface SavedToken {
  login: string;
  scope_warning: string | null;
}

// GitHub's new-token page accepts these query parameters and arrives with
// the scope pre-ticked and the description filled in. Undocumented GitHub
// behaviour: if it silently stops working the link degrades to a plain
// token page and the note copy carries the guidance alone.
const CREATE_TOKEN_URL =
  "https://github.com/settings/tokens/new?scopes=repo&description=Grapevine";

// Preset sync cadences; the backend accepts anything in 30s..1h, this is
// just the curated menu. A hand-edited settings.json value outside the list
// is appended as its own option so the select never lies about the state.
const POLL_PRESETS = [60, 120, 180, 300, 600, 900, 1800, 3600];

export function pollLabel(secs: number): string {
  if (secs === 3600) return "1 hour";
  if (secs < 60 || secs % 60 !== 0) return `${secs} seconds`;
  const minutes = secs / 60;
  return minutes === 1 ? "1 minute" : `${minutes} minutes`;
}

function SettingsView() {
  const [tokenStatus, setTokenStatus] = useState<TokenStatus>({
    has_token: false,
    login: null,
  });
  const [tokenInput, setTokenInput] = useState("");
  const [tokenError, setTokenError] = useState("");
  // Non-blocking scope notice from the last save; the token is stored even
  // when this is set. Feedback on the save action only, so it does not
  // reappear after a restart.
  const [tokenWarning, setTokenWarning] = useState("");
  const [tokenBusy, setTokenBusy] = useState(false);

  const [repos, setRepos] = useState<string[]>([]);
  const [repoInput, setRepoInput] = useState("");
  const [repoError, setRepoError] = useState("");
  const [repoBusy, setRepoBusy] = useState(false);

  // null until the stored value arrives, so the select never flashes a
  // default the user did not pick.
  const [pollSecs, setPollSecs] = useState<number | null>(null);
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  const [generalError, setGeneralError] = useState("");

  useEffect(() => {
    invoke<TokenStatus>("token_status").then(setTokenStatus).catch(() => {});
    invoke<string[]>("list_repos").then(setRepos).catch(() => {});
    invoke<number>("get_poll_interval").then(setPollSecs).catch(() => {});
    invoke<boolean>("get_launch_at_login").then(setLaunchAtLogin).catch(() => {});
  }, []);

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
    try {
      setRepos(await invoke<string[]>("add_repo", { name: repoInput }));
      setRepoInput("");
    } catch (error) {
      setRepoError(String(error));
    } finally {
      setRepoBusy(false);
    }
  }

  async function removeRepo(name: string) {
    setRepoError("");
    try {
      setRepos(await invoke<string[]>("remove_repo", { name }));
    } catch (error) {
      setRepoError(String(error));
    }
  }

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
          {repos.length === 0 ? (
            <p className="settings-row settings-empty">No repositories yet.</p>
          ) : (
            <ul className="repo-list">
              {repos.map((repo) => (
                <li key={repo} className="settings-row repo-row">
                  <span className="repo-name">{repo}</span>
                  <button
                    type="button"
                    className="repo-remove"
                    aria-label={`Remove ${repo}`}
                    onClick={() => removeRepo(repo)}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
          <form className="settings-row settings-input-row" onSubmit={addRepo}>
            <input
              value={repoInput}
              onChange={(event) => setRepoInput(event.target.value)}
              placeholder="owner/repo"
              autoComplete="off"
              spellCheck={false}
            />
            <button type="submit" disabled={repoBusy || repoInput.trim() === ""}>
              {repoBusy ? "Checking…" : "Add"}
            </button>
          </form>
        </div>
        {repoError && <p className="settings-error">{repoError}</p>}
      </section>

      <section className="settings-section">
        <h2 className="settings-label">General</h2>
        <div className="settings-card">
          <label className="settings-row settings-field-row">
            <span>Check for updates every</span>
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
    </main>
  );
}

export default SettingsView;
