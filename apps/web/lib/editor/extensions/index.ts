/**
 * The composed extension set (ADR 0007 — MIT-only binding policy).
 *
 * StarterKit v3 (OSS core) already bundles Link and Underline marks; we
 * add the remaining MIT OSS extensions explicitly (Image, TaskList/
 * TaskItem, YouTube) and the Marula OSS extensions (KB-embed, asset
 * link, taxonomy term). No Tiptap paid bundle ever enters the tree.
 * Core-kb's content allowlist (crates/core-kb/src/content.rs) matches
 * THIS set — adding an extension here means extending that allowlist in
 * the same change, never bypassing it.
 */
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import { Youtube } from "@tiptap/extension-youtube";

import { MarulaAssetLink } from "./asset-link";
import { MarulaKbEmbed } from "./kb-embed";
import { MarulaTaxonomyTerm } from "./taxonomy-term";

export { MarulaAssetLink } from "./asset-link";
export { MarulaKbEmbed, type MarulaKbEmbedAttrs } from "./kb-embed";
export { MarulaTaxonomyTerm } from "./taxonomy-term";
import type { Editor } from "@tiptap/core";
export type MarulaEditor = Editor;

/** The single composition point — the /kb editor and every future rich
 * surface loads this set (or a subset). */
export const marulaExtensionSet = [
  StarterKit.configure({
    heading: { levels: [2, 3, 4] },
    link: { openOnClick: false, autolink: true },
  }),
  Image.configure({ allowBase64: false }),
  TaskList,
  TaskItem.configure({ nested: true }),
  Youtube.configure({
    // Privacy-safe official embeds; the SERVER (core-kb) still validates
    // src against the ADR 0004 registry on every save — the extension
    // only shapes docs; enforcement never trusts the client.
    nocookie: true,
    allowFullscreen: true,
    controls: true,
    width: 640,
    height: 360,
  }),
  MarulaAssetLink,
  MarulaKbEmbed,
  MarulaTaxonomyTerm,
];
