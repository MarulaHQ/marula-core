// Local-date helpers for flow authors.
//
// The API and the UI both speak LOCAL calendar dates for day-scoped fields
// (due dates, effective ranges, the day pack). Formatting a Date with
// `toISOString().slice(0, 10)` converts to UTC first, so east of UTC it
// reports YESTERDAY for the two hours after local midnight — a suite run at
// 00:25 SAST filled "tomorrow" with today's date and asserted "Due
// tomorrow", which the UI renders as "Due today". These helpers keep the
// date string in the local calendar, matching what the product renders.

/** `YYYY-MM-DD` in the local calendar. */
export function localDate(d: Date = new Date()): string {
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** `YYYY-MM-DD` for `days` from today, in the local calendar. */
export function localDatePlus(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return localDate(d);
}
