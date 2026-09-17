// Shared note-body formatter — the client mirror of the ONE server
// implementation (`core-domain::notes::format_note_body`, Wave 17 lane 11,
// docs/notes-tasks-workbar.md §2.4). The rules are duplicated ONLY as the
// typed-connection mirror the spec mandates: the server composes
// auto-appends, the client normalizes drafts at submit. Both are
// property-tested for B6 idempotence (server: proptest; client: vitest).
//
// Rules, in order: B1 trim/collapse blanks · B2 canonical `- ` bullets
// (typed markers normalized; ordered numerals preserved) · B3 ≤3-space
// indent kept under the bullet · B4 first-letter capitalization · L1/L2
// links verbatim · L3 bare-URL lines gain https://. No terminal
// punctuation (B5).

export function formatNoteBody(draft: string): string {
  const lines: string[] = [];
  for (const raw of draft.split(/\r?\n/)) {
    if (raw.trim() === "") continue; // B1: blank lines never survive
    // B3: up to 3 leading spaces of intentional indentation survive;
    // 4+ is flow text — indent resets to zero, whitespace goes.
    const leading = raw.length - raw.trimStart().length;
    const [indent, content] =
      leading > 3 ? [0, raw.trimStart()] : [leading, raw.slice(leading)];
    // B2: normalize typed bullet markers to the canonical `- ` marker.
    const markerStripped = content
      .replace(/^[ \t]+/, "")
      .replace(/^- (?!\s*-)/, "")
      .replace(/^\* /, "")
      .replace(/^• /, "")
      .replace(/^– /, "");
    // B2 exception: ordered numerals are PRESERVED as typed.
    const numerable = isOrderedNumeral(markerStripped);
    // L3: a line consisting SOLELY of a bare host URL gets https://.
    let text =
      !numerable && !/\s/.test(markerStripped) && looksLikeBareHost(markerStripped)
        ? `https://${markerStripped}`
        : markerStripped;
    // B4: first ALPHABETIC character is uppercased; digits untouched.
    // Verbatim exceptions: raw URLs, markdown links, markdown bold leads.
    const skipCap =
      text.startsWith("http://") ||
      text.startsWith("https://") ||
      text.startsWith("[") ||
      text.startsWith("**");
    if (!skipCap) {
      const idx = text.search(/[A-Za-zÀ-ÖØ-öø-ÿẞ]/);
      const ch = idx >= 0 ? text[idx] : undefined;
      if (idx >= 0 && ch !== undefined) {
        text =
          text.slice(0, idx) +
          ch.toUpperCase() +
          text.slice(idx + 1);
      }
    }
    lines.push(numerable ? `${" ".repeat(indent)}${text}` : `${" ".repeat(indent)}- ${text}`);
  }
  return lines.join("\n");
}

function isOrderedNumeral(line: string): boolean {
  // Mirrors the server: "1. " / "12) " prefixes only.
  return /^(\d+)(\. |\) )/.test(line);
}

function looksLikeBareHost(line: string): boolean {
  if (line.startsWith("[") || line.startsWith("http://") || line.startsWith("https://")) {
    return false;
  }
  const head = line.split(/[ /]/)[0] ?? "";
  return (
    head.includes(".") && !head.startsWith(".") && !head.endsWith(".")
  );
}

/** The avatar initials rule (§2.2): first+last initials of a display
 * name; single-name users get the first two letters. */
export function initialsOf(displayName: string | undefined): string {
  const name = (displayName ?? "").trim();
  if (name === "") return "··";
  const parts = name.split(/\s+/).filter((p) => p !== "");
  const first = parts[0]?.[0];
  if (parts.length >= 2 && first !== undefined && parts[parts.length - 1]?.[0] !== undefined) {
    return (first + parts[parts.length - 1]![0]!).toUpperCase();
  }
  return name.slice(0, 2).toUpperCase();
}
