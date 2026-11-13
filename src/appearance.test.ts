import { afterEach, expect, test } from "vitest";
import { createAppearance, type Theme, type ThemeHost } from "./appearance";
import { EMPTY_SNAPSHOT, getAppearance, setAdapter, setAppearance } from "./backend";
import { createFakeBackend } from "./backend/fake";
import { tauriAdapter } from "./backend/tauri";

afterEach(() => setAdapter(tauriAdapter));

// A desktop whose style the test flips, and a record of every theme shown.
function desktop(dark: boolean) {
  const shown: Theme[] = [];
  const listeners: (() => void)[] = [];
  const host: ThemeHost = {
    systemIsDark: () => dark,
    onSystemChange: (listener) => listeners.push(listener),
    show: (theme) => shown.push(theme),
  };
  const switchTo = (next: "light" | "dark") => {
    dark = next === "dark";
    for (const listener of listeners) listener();
  };
  return { host, shown, switchTo };
}

const useFakeBackend = () => setAdapter(createFakeBackend(EMPTY_SNAPSHOT).adapter);

test("with nothing chosen the popover takes the desktop's style", async () => {
  useFakeBackend();
  const { host, shown } = desktop(true);
  await createAppearance(host).start();
  expect(shown[shown.length - 1]).toBe("dark");
});

test("the desktop's style shows before the stored choice has loaded", () => {
  useFakeBackend();
  const { host, shown } = desktop(true);
  void createAppearance(host).start();
  expect(shown).toEqual(["dark"]);
});

test("a stored choice overrides the desktop's style", async () => {
  useFakeBackend();
  await setAppearance("light");
  const { host, shown } = desktop(true);
  await createAppearance(host).start();
  expect(shown[shown.length - 1]).toBe("light");
});

test("System follows the desktop when it switches style", async () => {
  useFakeBackend();
  const { host, shown, switchTo } = desktop(false);
  await createAppearance(host).start();
  switchTo("dark");
  expect(shown[shown.length - 1]).toBe("dark");
  switchTo("light");
  expect(shown[shown.length - 1]).toBe("light");
});

test("a fixed choice holds when the desktop switches style", async () => {
  useFakeBackend();
  await setAppearance("light");
  const { host, shown, switchTo } = desktop(false);
  await createAppearance(host).start();
  switchTo("dark");
  expect(shown[shown.length - 1]).toBe("light");
});

test("choosing an appearance stores it and shows it at once", async () => {
  useFakeBackend();
  const { host, shown } = desktop(false);
  const appearance = createAppearance(host);
  await appearance.start();
  expect(await appearance.choose("dark")).toBe("dark");
  expect(shown[shown.length - 1]).toBe("dark");
  expect(await getAppearance()).toBe("dark");
});

test("choosing System again goes back to the desktop's style", async () => {
  useFakeBackend();
  await setAppearance("light");
  const { host, shown } = desktop(true);
  const appearance = createAppearance(host);
  await appearance.start();
  await appearance.choose("system");
  expect(shown[shown.length - 1]).toBe("dark");
});

test("a choice the backend refuses changes nothing on screen", async () => {
  useFakeBackend();
  const { host, shown } = desktop(false);
  const appearance = createAppearance(host);
  await appearance.start();
  setAdapter({
    ...tauriAdapter,
    invoke: async () => {
      throw "disk full";
    },
  });
  await expect(appearance.choose("dark")).rejects.toBe("disk full");
  expect(shown[shown.length - 1]).toBe("light");
});

// A settings file that cannot be read must not leave the popover unthemed.
test("settings that cannot be read leave the popover on the desktop's style", async () => {
  setAdapter({
    ...tauriAdapter,
    invoke: async () => {
      throw "settings.json is corrupt";
    },
  });
  const { host, shown, switchTo } = desktop(true);
  await createAppearance(host).start();
  expect(shown[shown.length - 1]).toBe("dark");
  switchTo("light");
  expect(shown[shown.length - 1]).toBe("light");
});

// React's StrictMode mounts the app twice in development.
test("starting twice follows the desktop once", async () => {
  useFakeBackend();
  const { host, shown, switchTo } = desktop(false);
  const appearance = createAppearance(host);
  await Promise.all([appearance.start(), appearance.start()]);
  const before = shown.length;
  switchTo("dark");
  expect(shown.slice(before)).toEqual(["dark"]);
});
