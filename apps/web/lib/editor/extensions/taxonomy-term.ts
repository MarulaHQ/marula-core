/**
 * Meridian taxonomy term (note-taxonomy picker) — inline node carrying a
 * term id; the canvas renders an inline chip. Term resolution and
 * visibility stay server-side.
 */
import { Node } from "@tiptap/core";

export const MeridianTaxonomyTerm = Node.create({
  name: "meridianTaxonomyTerm",
  group: "inline",
  inline: true,
  atom: true,

  addAttributes() {
    return {
      termId: {
        default: null,
        parseHTML: (element) => element.getAttribute("data-term-id"),
      },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-meridian-taxonomy-term]" }];
  },

  renderHTML({ node }) {
    return [
      "span",
      {
        "data-meridian-taxonomy-term": "true",
        "data-term-id": (node.attrs.termId as string) ?? "",
        class: "kb-taxonomy-term",
      },
      "term",
    ];
  },
});
