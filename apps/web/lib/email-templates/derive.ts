// Draft-side text helpers for the template editor: placeholder extraction,
// draft rendering (client-side, sample values), and HTML → plain text.
//
// These run in the browser on the editor's own draft — the server's render
// endpoint (`POST /api/v1/templates/{id}/render`) resolves the SAVED
// template against a REAL entity, so a draft preview substitutes locally
// instead. Unknown placeholders are left visible in the preview rather than
// silently emptied (the server's own rule: a variable can never expand
// silently to nothing).

/** Every `{{key}}` placeholder in `text`, first-occurrence order, trimmed. */
export function extractPlaceholders(text: string): string[] {
  const keys: string[] = [];
  for (const match of text.matchAll(/\{\{([^{}]+)\}\}/g)) {
    const key = (match[1] ?? "").trim();
    if (key !== "" && !keys.includes(key)) keys.push(key);
  }
  return keys;
}

/**
 * Substitutes `{{key}}` placeholders from `values`. A key the values don't
 * carry stays as-is, so the editor can never show a preview that hides a
 * missing variable.
 */
export function renderDraft(text: string, values: Record<string, string>): string {
  return text.replace(/\{\{([^{}]+)\}\}/g, (whole, raw: string) => {
    const key = raw.trim();
    const value = values[key];
    return value !== undefined ? value : whole;
  });
}

/**
 * Strips HTML down to plain text for the fallback view. Block-level tags
 * become line breaks so paragraphs survive; entities decode through the
 * browser's own parser. Good enough for an auto-derived fallback — the
 * marked honest affordance, not a hand-tuned email plaintext.
 */
export function htmlToPlainText(html: string): string {
  if (html.trim() === "") return "";
  const doc = new DOMParser().parseFromString(html, "text/html");
  const breaks = new Set(["BR", "P", "DIV", "LI", "TR", "H1", "H2", "H3", "H4", "BLOCKQUOTE"]);
  let out = "";
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.textContent ?? "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const element = node as Element;
    if (element.tagName === "SCRIPT" || element.tagName === "STYLE") return;
    const block = breaks.has(element.tagName);
    if (block) out += "\n";
    for (const child of element.childNodes) walk(child);
    if (element.tagName === "LI") out += "\n";
  };
  walk(doc.body);
  // Two blank lines never help an email; one paragraph break does.
  return out.replace(/\n{3,}/g, "\n\n").trim();
}
