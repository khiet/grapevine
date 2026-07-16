import { expect, test } from "vitest";
import { formatAge } from "./PrList";

const NOW = Date.parse("2026-07-16T00:00:00Z");
const secondsAgo = (seconds: number) =>
  new Date(NOW - seconds * 1000).toISOString();

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

test("ages under a minute render as now", () => {
  expect(formatAge(secondsAgo(0), NOW)).toBe("now");
  expect(formatAge(secondsAgo(59), NOW)).toBe("now");
});

test("timestamps slightly in the future render as now, not negative", () => {
  expect(formatAge(secondsAgo(-30), NOW)).toBe("now");
});

test("each unit takes over at its boundary", () => {
  expect(formatAge(secondsAgo(MINUTE), NOW)).toBe("1m");
  expect(formatAge(secondsAgo(59 * MINUTE), NOW)).toBe("59m");
  expect(formatAge(secondsAgo(HOUR), NOW)).toBe("1h");
  expect(formatAge(secondsAgo(23 * HOUR), NOW)).toBe("23h");
  expect(formatAge(secondsAgo(DAY), NOW)).toBe("1d");
  expect(formatAge(secondsAgo(6 * DAY), NOW)).toBe("6d");
  expect(formatAge(secondsAgo(WEEK), NOW)).toBe("1w");
  expect(formatAge(secondsAgo(51 * WEEK), NOW)).toBe("51w");
});

test("52 weeks rounds to a year rather than 0y", () => {
  expect(formatAge(secondsAgo(52 * WEEK), NOW)).toBe("1y");
  expect(formatAge(secondsAgo(104 * WEEK), NOW)).toBe("2y");
});
