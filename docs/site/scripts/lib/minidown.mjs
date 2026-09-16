// minidown.mjs — the docs-site publisher's deterministic markdown renderer.
//
// Wave 15 lane 4 (`p15-docs-closeout`): the publishing decision (ADR 0009)
// keeps `docs/site` markdown as the SOURCE OF TRUTH and renders the
// publication in-repo with zero dependencies — no Starlight fork, no extra
// framework. This module does NOT fork the markdown sources; it only renders
// them, so the repo-native sources stay canonical and `check-docs-site.mjs`
// still verifies their links and generator freshness.
//
// Scope is deliberately narrow: exactly the constructs the site pages use
// (headings, paragraphs, bullet/ordered lists, tables, blockquotes, fenced
// code, HTML comments, inline code/bold/italic/links). Anything a new page
// needs that this lacks must be added here — never a second renderer per
// consumer. Output is byte-deterministic (no timestamps, no locale).

/**
 * Renders one markdown source into an HTML fragment (body content only;
 * the caller owns the page shell).
 *
 * @param {string} src raw markdown text
 * @param {{href?: (target: string) => string|null}} [opts]
 *   href(target): maps a link target to its published href, or null to
 *   render the link as plain text (dropped references). Absent = targets
 *   pass through unmodified.
 * @returns {string} HTML
 */
export function render(src, opts = {}) {
  return new Doc(src.replace(/\r\n?/g, "\n"), opts).render();
}

const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };
/** Escaping for text content (quotes pass through for readability). */
const escapeText = (s) => s.replace(/[&<>]/g, (c) => ESCAPES[c]);
/** Escaping for attribute values (quoting matters there). */
const escapeAttr = (s) => s.replace(/[&<>"]/g, (c) => ESCAPES[c]);

function slugify(text) {
  return (
    text
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "section"
  );
}

class Doc {
  constructor(src, opts) {
    this.lines = src.split("\n");
    this.opts = opts;
    this.out = [];
  }

  render() {
    let i = 0;
    while (i < this.lines.length) {
      const raw = this.lines[i];

      if (raw.trim() === "") {
        i++;
        continue;
      }
      // HTML comment block (generator stamps) — carried out of the render.
      if (raw.trimStart().startsWith("<!--")) {
        i++;
        while (
          i < this.lines.length &&
          !raw.includes("-->") &&
          !this.lines[i].includes("-->")
        ) {
          i++;
        }
        if (i < this.lines.length && this.lines[i].includes("-->")) i++;
        continue;
      }
      // Fenced code.
      const fenceMatch = raw.match(/^(`{3,})(.*)$/);
      if (fenceMatch) {
        const lang = fenceMatch[2].trim();
        const body = [];
        i++;
        while (i < this.lines.length && !this.lines[i].startsWith(fenceMatch[1])) {
          body.push(this.lines[i]);
          i++;
        }
        i++; // closing fence
        const cls = lang ? ` class="language-${escapeAttr(lang)}"` : "";
        this.out.push(`<pre><code${cls}>${escapeText(body.join("\n"))}\n</code></pre>\n`);
        continue;
      }
      // Headings.
      const heading = raw.match(/^(#{1,4})\s+(.*)$/);
      if (heading) {
        const level = heading[1].length;
        const text = this.inline(heading[2]);
        const id = slugify(heading[2].replace(/[*`]/g, ""));
        this.out.push(`<h${level} id="${id}">${text}</h${level}>\n`);
        i++;
        continue;
      }
      // Table: header row + separator row.
      if (
        raw.trimStart().startsWith("|") &&
        i + 1 < this.lines.length &&
        /^\s*\|?[\s:|-]+\|?\s*$/.test(this.lines[i + 1]) &&
        this.lines[i + 1].includes("-")
      ) {
        const rows = [];
        const header = this.splitRow(raw).map((c) => this.inline(c));
        i += 2; // skip header + separator
        while (i < this.lines.length && this.lines[i].trimStart().startsWith("|")) {
          rows.push(this.lines[i]);
          i++;
        }
        let html = "<table>\n<thead>\n<tr>";
        for (const c of header) html += `<th>${c}</th>`;
        html += "</tr>\n</thead>\n<tbody>\n";
        for (const rowLine of rows) {
          html += "<tr>";
          for (const c of this.splitRow(rowLine)) html += `<td>${this.inline(c.trim())}</td>`;
          html += "</tr>\n";
        }
        html += "</tbody>\n</table>\n";
        this.out.push(html);
        continue;
      }
      // Blockquote.
      if (raw.trimStart().startsWith(">")) {
        const body = [];
        while (i < this.lines.length && this.lines[i].trimStart().startsWith(">")) {
          body.push(this.lines[i].trimStart().replace(/^>\s?/, ""));
          i++;
        }
        this.out.push(`<blockquote><p>${this.inline(body.join(" "))}</p></blockquote>\n`);
        continue;
      }
      // Bullet lists.
      if (/^\s*[-*]\s/.test(raw) && !raw.startsWith("    ")) {
        const items = [];
        while (i < this.lines.length && /^\s*[-*]\s/.test(this.lines[i])) {
          items.push(this.lines[i].trim().slice(2));
          i++;
          // Indented continuation lines join the open item (soft wrap).
          while (
            i < this.lines.length &&
            this.lines[i].trim() !== "" &&
            /^\s{2,}[^-*\d]/.test(this.lines[i])
          ) {
            items[items.length - 1] += ` ${this.lines[i].trim()}`;
            i++;
          }
        }
        this.out.push(
          `<ul>\n${items.map((it) => `  <li>${this.inline(it)}</li>`).join("\n")}\n</ul>\n`,
        );
        continue;
      }
      // Ordered lists.
      if (/^\s*\d+\.\s/.test(raw) && !raw.startsWith("    ")) {
        const items = [];
        while (i < this.lines.length && /^\s*\d+\.\s/.test(this.lines[i])) {
          items.push(this.lines[i].trim().replace(/^\d+\.\s*/, ""));
          i++;
          while (
            i < this.lines.length &&
            this.lines[i].trim() !== "" &&
            /^\s{2,}[^-*\d]/.test(this.lines[i])
          ) {
            items[items.length - 1] += ` ${this.lines[i].trim()}`;
            i++;
          }
        }
        this.out.push(
          `<ol>\n${items.map((it) => `  <li>${this.inline(it)}</li>`).join("\n")}\n</ol>\n`,
        );
        continue;
      }
      // Paragraph: collect until blank line or a block start.
      const para = [];
      while (
        i < this.lines.length &&
        this.lines[i].trim() !== "" &&
        !this.lines[i].trimStart().startsWith("<!--") &&
        !/^`{3,}/.test(this.lines[i]) &&
        !/^(#{1,4})\s/.test(this.lines[i]) &&
        !this.lines[i].trimStart().startsWith(">") &&
        !/^\s*[-*]\s/.test(this.lines[i]) &&
        !/^\s*\d+\.\s/.test(this.lines[i])
      ) {
        para.push(this.lines[i].trim());
        i++;
      }
      if (para.length) {
        this.out.push(`<p>${this.inline(para.join(" "))}</p>\n`);
      }
    }
    return this.out.join("");
  }

  splitRow(line) {
    const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
    return trimmed.split(/(?<!\\)\|/).map((c) => c.trim());
  }

  /**
   * Inline rendering with a precedence scan: code spans and link anchors
   * are pulled out earliest (so a code-only label never breaks the link),
   * then text runs get escaping + bold + italic. Anchor labels recurse
   * without a second link scan.
   */
  inline(src) {
    const parts = [];
    let rest = src;
    for (;;) {
      const anchor = rest.match(/\[([^\]]*)\]\(([^)\s]+)\)/);
      const code = rest.match(/`([^`]+)`/);
      const kind =
        anchor && code
          ? anchor.index <= code.index ? "anchor" : "code"
          : anchor ? "anchor"
          : code ? "code"
          : null;
      if (!kind) break;
      const match = kind === "anchor" ? anchor : code;
      parts.push(rest.slice(0, match.index));
      parts.push(
        kind === "anchor"
          ? { anchor: [anchor[1], anchor[2]] }
          : { code: code[1] },
      );
      rest = rest.slice(match.index + match[0].length);
    }
    parts.push(rest);

    return parts
      .map((part) => {
        if (typeof part === "string") return this.freestanding(part);
        if (part.code) return `<code>${escapeText(part.code)}</code>`;
        const [label, target] = part.anchor;
        const mapped = this.opts.href ? this.opts.href(target) : target;
        if (mapped === null) return this.inline(label);
        const title = escapeAttr(decodeURIComponent(target.split("#")[0]));
        return `<a href="${escapeAttr(mapped)}" title="${title}">${this.inline(label)}</a>`;
      })
      .join("");
  }

  /** Escape + bold + italic for a text run. */
  freestanding(text) {
    const escaped = escapeText(text);
    let html = escaped.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    html = html.replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
    return html;
  }
}
