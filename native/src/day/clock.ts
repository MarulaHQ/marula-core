// Clock-entry parsing for the travel form, kept free of React Native so the
// rules stay unit-testable.

/** "08:30" for a local timestamp — the prefill the form opens with. */
export function clockInput(epochMs: number): string {
  const date = new Date(epochMs);
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** Parses "H:MM" / "HH:MM" into a Date on `day` (today by default).
 * Returns null for anything a technician could not have meant. */
export function parseClock(value: string, day: Date = new Date()): Date | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (match === null) return null;
  const hours = Number.parseInt(match[1]!, 10);
  const minutes = Number.parseInt(match[2]!, 10);
  if (hours > 23 || minutes > 59) return null;
  const out = new Date(day);
  out.setHours(hours, minutes, 0, 0);
  return out;
}
