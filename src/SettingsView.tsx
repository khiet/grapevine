import { FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface TokenStatus {
  has_token: boolean;
  login: string | null;
}

function SettingsView() {
  const [tokenStatus, setTokenStatus] = useState<TokenStatus>({
    has_token: false,
    login: null,
  });
  const [tokenInput, setTokenInput] = useState("");
  const [tokenError, setTokenError] = useState("");
  const [tokenBusy, setTokenBusy] = useState(false);

  const [repos, setRepos] = useState<string[]>([]);
  const [repoInput, setRepoInput] = useState("");
  const [repoError, setRepoError] = useState("");
  const [repoBusy, setRepoBusy] = useState(false);

  useEffect(() => {
    invoke<TokenStatus>("token_status").then(setTokenStatus).catch(() => {});
    invoke<string[]>("list_repos").then(setRepos).catch(() => {});
  }, []);

  async function saveToken(event: FormEvent) {
    event.preventDefault();
    setTokenBusy(true);
    setTokenError("");
    try {
      const login = await invoke<string>("save_token", { token: tokenInput });
      setTokenStatus({ has_token: true, login });
      setTokenInput("");
    } catch (error) {
      setTokenError(String(error));
    } finally {
      setTokenBusy(false);
    }
  }

  async function removeToken() {
    setTokenError("");
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
        {tokenStatus.has_token && (
          <div className="settings-status-row">
            <span className="settings-ok">
              Signed in as <strong>{tokenStatus.login ?? "unknown"}</strong>
            </span>
            <button type="button" className="settings-link" onClick={removeToken}>
              Remove
            </button>
          </div>
        )}
        <form className="settings-input-row" onSubmit={saveToken}>
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
        {tokenError && <p className="settings-error">{tokenError}</p>}
      </section>

      <section className="settings-section">
        <h2 className="settings-label">Watched repositories</h2>
        {repos.length === 0 ? (
          <p className="settings-empty">No repositories yet.</p>
        ) : (
          <ul className="repo-list">
            {repos.map((repo) => (
              <li key={repo} className="repo-row">
                <span className="repo-name">{repo}</span>
                <button
                  type="button"
                  className="settings-link"
                  aria-label={`Remove ${repo}`}
                  onClick={() => removeRepo(repo)}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
        <form className="settings-input-row" onSubmit={addRepo}>
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
        {repoError && <p className="settings-error">{repoError}</p>}
      </section>
    </main>
  );
}

export default SettingsView;
