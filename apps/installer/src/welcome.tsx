/**
 * The first thing the installer shows.
 *
 * Purpose: a person double-clicked an installer. Before asking five screens of
 * questions, the app says what it is and what it will not do without permission.
 *
 * The background is one of the brand's four textures, chosen once per launch so
 * the app is recognisably the same product without being the same picture twice.
 * It sits behind a scrim rather than behind text: the artwork is patterned, and
 * type on a pattern is unreadable at any contrast ratio the tokens allow.
 */

import { useState } from "react";
import { Brand } from "./brand";
import ndebele from "./assets/brand/ndebele-inspired-wall-1200x800.png";
import gouache from "./assets/brand/orchard-gouache-1600x900.jpg";
import shweshwe from "./assets/brand/shweshwe-inspired-indigo-1200x800.png";
import terracotta from "./assets/brand/terracotta-zellige-1200x800.png";

// Imported rather than served from `public/`: the loopback server hands out its
// own asset directory and nothing else, so a root-relative path 404s there while
// working in a dev server. Through the bundler both hosts get the same files,
// with a content hash and the right size.
const BACKGROUNDS = [shweshwe, ndebele, terracotta, gouache];

export function Welcome({ onStart }: { onStart: () => void }) {
  // Once per launch, not once per render: a texture that changed under the
  // pointer would read as a glitch.
  const [background] = useState(
    () => BACKGROUNDS[Math.floor(Math.random() * BACKGROUNDS.length)] ?? BACKGROUNDS[0]!,
  );

  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden p-6">
      <img
        src={background}
        alt=""
        aria-hidden
        className="absolute inset-0 h-full w-full object-cover"
      />
      <div className="absolute inset-0 bg-background/75" />

      <div className="relative w-full max-w-[520px] rounded-card border border-border bg-card p-9 text-center shadow-sm">
        <Brand className="mx-auto h-11" />
        <h1 className="mt-6 mb-0 font-display text-[30px] font-normal leading-[1.15] tracking-[-0.01em]">
          Set up Marula on this machine
        </h1>
        <p className="mx-auto mt-3 mb-0 max-w-[38ch] text-[13.5px] leading-[1.55] text-muted-foreground">
          Five screens, one machine. Nothing is written until you have seen the plan and
          said so.
        </p>
        <button
          type="button"
          onClick={onStart}
          className="mt-7 rounded-ctl bg-primary px-4 py-2.5 text-[13px] font-semibold text-primary-foreground"
        >
          Get started
        </button>
        <p className="m-0 mt-5 font-mono text-[10.5px] uppercase tracking-[0.14em] text-muted-foreground/80">
          no telemetry · runs on this machine
        </p>
      </div>
    </div>
  );
}
