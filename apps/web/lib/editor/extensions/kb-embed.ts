/**
 * Meridian KB-embed node (ADR 0007 scope; OSS Tiptap extension — MIT,
 * authored in-house). The working canvas renders a placeholder card;
 * the READ path renders from core-kb's sanitized renderer, never this
 * shim. Embed URL policy stays server-side (rich.rs registry) — the
 * node only SHAPES data; no script/iframe shapes serialize from here.
 */
import { Node } from "@tiptap/core";

export interface MeridianKbEmbedAttrs {
  src: string;
  title?: string;
}

export const MeridianKbEmbed = Node.create({
  name: "meridianKbEmbed",
  group: "block",
  atom: true,
  draggable: true,

  addAttributes() {
    return {
      src: {
        default: null as string | null,
        parseHTML: (element) => element.getAttribute("data-src"),
      },
      title: {
        default: null,
        parseHTML: (element) => element.getAttribute("data-title"),
      },
    };
  },

  parseHTML() {
    return [{ tag: 'figure[data-marula-embed="kb"]' }];
  },

  renderHTML({ node }) {
    return [
      "figure",
      {
        "data-marula-embed": "kb",
        class: "kb-embed",
      },
      [
        "figcaption",
        {},
        (node.attrs.title as string) ?? "Embed card",
      ],
    ];
  },
});
