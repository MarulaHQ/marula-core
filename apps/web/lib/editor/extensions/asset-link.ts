/**
 * Meridian asset link (RelatedAssetLink / RecordLink, ADR 0006) — an
 * inline node referencing one asset id. The Canvas shows an inline
 * chip; read rendering (core-kb) carries the resolved/broken state as
 * classes. Data-only node: resolution lives server-side.
 */
import { Node } from "@tiptap/core";

export const MeridianAssetLink = Node.create({
  name: "meridianAssetLink",
  group: "inline",
  inline: true,
  atom: true,

  addAttributes() {
    return {
      assetId: {
        default: null,
        parseHTML: (element) => element.getAttribute("data-asset-id"),
      },
    };
  },

  parseHTML() {
    return [{ tag: 'span[data-meridian-asset-link]' }];
  },

  renderHTML({ node }) {
    return [
      "span",
      {
        "data-meridian-asset-link": "true",
        "data-asset-id": (node.attrs.assetId as string) ?? "",
        class: "kb-asset-link",
      },
      node.textContent || "Linked asset",
    ];
  },
});
