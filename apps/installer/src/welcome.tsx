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
 *
 * The appearance control is in this screen's own header rather than inherited
 * from the wizard's rail, because the rail arrives one click later and someone
 * on a light desktop meets this screen first. It is the rail's own three-way
 * choice, not a second theme model: a sun/moon switch here would have silently
 * dropped `system`.
 */

import { Segmented, SegmentedItem } from "@marula-core/ui/kit";
import { useState } from "react";
import { Brand } from "./brand";
import ndebele from "./assets/textures/ndebele-inspired-wall-1200x800.webp";
import gouache from "./assets/textures/orchard-gouache-1600x900.webp";
import shweshwe from "./assets/textures/shweshwe-inspired-indigo-1200x800.webp";
import terracotta from "./assets/textures/terracotta-zellige-1200x800.webp";

// Imported rather than served from `public/`: the loopback server hands out its
// own asset directory and nothing else, so a root-relative path 404s there while
// working in a dev server. Through the bundler both hosts get the same files,
// with a content hash and the right size.
const BACKGROUNDS = [shweshwe, ndebele, terracotta, gouache];

type Appearance = "system" | "light" | "dark";

const APPEARANCE: Appearance[] = ["system", "light", "dark"];

export function Welcome({
  onStart,
  theme,
  onTheme,
}: {
  onStart: () => void;
  theme: Appearance;
  onTheme: (theme: Appearance) => void;
}) {
  // Once per launch, not once per render: a texture that changed under the
  // pointer would read as a glitch.
  const [background] = useState(
    () => BACKGROUNDS[Math.floor(Math.random() * BACKGROUNDS.length)] ?? BACKGROUNDS[0]!,
  );

  return (
    <div className="relative flex h-screen flex-col overflow-hidden">
      <img
        src={background}
        alt=""
        aria-hidden
        className="absolute inset-0 h-full w-full object-cover"
      />
      <div className="absolute inset-0 bg-background/75" />

      {/* The header keeps the control off the card, in the same place the login
          page puts its one action: top right, on the surface, ringed rather than
          shadowed so it holds on a pattern. */}
      <header className="relative flex items-center justify-end px-5 py-4 lg:px-7 lg:py-5">
        <Segmented
          type="single"
          value={theme}
          onValueChange={(value) => value && onTheme(value as Appearance)}
          aria-label="Appearance"
          className="bg-card/95 shadow-sm ring-1 ring-border"
        >
          {APPEARANCE.map((choice) => (
            <SegmentedItem
              key={choice}
              value={choice}
              size="sm"
              className="px-2.5 text-[11.5px] capitalize"
            >
              {choice}
            </SegmentedItem>
          ))}
        </Segmented>
      </header>

      <div className="relative flex flex-1 items-center justify-center px-6 pb-8">
        <div className="relative w-full max-w-[520px] rounded-card border border-border bg-card p-9 text-center shadow-sm">
          <Brand className="mx-auto h-11" />
          <h1 className="mt-6 mb-0 font-display text-[30px] font-normal leading-[1.15] tracking-[-0.01em]">
            Set up Marula on this machine
          </h1>
          <p className="mx-auto mt-3 mb-0 max-w-[46ch] text-[13.5px] leading-[1.55] text-muted-foreground">
            This wizard will guide you through the setup. You can choose between recommended settings to get you up and running quickly, or developer mode which will give you all the options.
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
    </div>
  );
}
