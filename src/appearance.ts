import { getAppearance, setAppearance, type Appearance } from "./backend";

export type Theme = "light" | "dark";

/** Where a theme is shown and where the desktop's style is read from. The
 * browser is the only real one; tests supply their own. */
export interface ThemeHost {
  systemIsDark(): boolean;
  onSystemChange(listener: () => void): void;
  show(theme: Theme): void;
}

export interface AppearanceControl {
  /** Shows the desktop's style at once, then the stored choice when it
   * arrives. Safe to call more than once; later calls change nothing. */
  start(): Promise<void>;
  /** Stores the choice, then shows it. Rejects, with the screen unchanged,
   * if the backend refuses. Resolves to the choice as stored. */
  choose(appearance: Appearance): Promise<Appearance>;
}

export function createAppearance(host: ThemeHost): AppearanceControl {
  let current: Appearance = "system";
  // Set by choose, so a stored value that loads late cannot undo it.
  let chosen = false;
  let started: Promise<void> | null = null;

  const show = () =>
    host.show(current === "system" ? (host.systemIsDark() ? "dark" : "light") : current);

  return {
    start() {
      if (started) return started;
      show();
      host.onSystemChange(show);
      started = getAppearance()
        .then((stored) => {
          if (chosen) return;
          current = stored;
          show();
        })
        // Unreadable settings leave the popover following the desktop.
        .catch(() => {});
      return started;
    },
    async choose(appearance) {
      const stored = await setAppearance(appearance);
      chosen = true;
      current = stored;
      show();
      return stored;
    },
  };
}

const PREFERS_DARK = "(prefers-color-scheme: dark)";

// App.css keys the dark palette off this attribute on the root element.
const browserHost: ThemeHost = {
  systemIsDark: () => window.matchMedia(PREFERS_DARK).matches,
  onSystemChange: (listener) =>
    window.matchMedia(PREFERS_DARK).addEventListener("change", listener),
  show: (theme) => {
    document.documentElement.dataset.theme = theme;
  },
};

/** The popover's own appearance. */
export const appearance = createAppearance(browserHost);
