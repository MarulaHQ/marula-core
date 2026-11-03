/**
 * The five screens. Each asks one thing, and each keeps the descriptor's long
 * explanations behind a disclosure instead of in the reading path.
 */

import { useMemo, useState } from "react";
import {
  Banner,
  Button,
  Callout,
  Card,
  CardBody,
  CardHeader,
  CardTitle,
  Field,
  Icon,
  Input,
  Kbd,
  Pill,
  Segmented,
  SegmentedItem,
  Select,
  Switch,
} from "@marula-core/ui/kit";

import {
  curatedFields,
  effectiveValue,
  floorRows,
  formatMb,
  humanLabel,
  paramsByGroup,
  searchParams,
  servicesLine,
  shapeLine,
  sourceCounts,
  warningsFor,
  type PathMode,
} from "./lib/model";
import { PlanList, Proof, Section, ValueTable } from "./ui";
import type { ApplyState, Check, Doctor, HostFacts, Param, Plan, Profile, State } from "./types";

/* ---------------------------------------------------------------- 1. path */

export function PathStep({
  chosen,
  onSelect,
  paramCount,
}: {
  chosen: "recommended" | "developer" | null;
  onSelect: (mode: "recommended" | "developer") => void;
  paramCount: number;
}) {
  return (
    <div className="grid max-w-[900px] grid-cols-1 items-stretch gap-4 md:grid-cols-2">
      <button
        type="button"
        onClick={() => onSelect("recommended")}
        aria-pressed={chosen === "recommended"}
        className={`rounded-card text-left transition-[box-shadow,transform] duration-100 ${
          chosen === "recommended"
            ? "ring-[1.5px] ring-[var(--mc-gold)]"
            : "ring-1 ring-border hover:ring-muted-foreground"
        }`}
      >
        <Card className="h-full">
          <CardHeader>
            <CardTitle>Recommended</CardTitle>
            <Pill tone="gold" className="ml-auto">
              few questions
            </Pill>
          </CardHeader>
          <CardBody className="gap-3">
            <p className="m-0 text-[13px] leading-[1.55] text-muted-foreground">
              Marula decides everything except the questions only you can answer.
            </p>
            <ul className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13px]">
              {["This machine, and the shape it can carry", "The address your team opens", "Where search and files live", "Review, then apply"].map(
                (item) => (
                  <li key={item} className="flex items-start gap-2">
                    <Icon name="check" size={14} className="mt-0.5 shrink-0 text-[var(--mc-green)]" />
                    <span>{item}</span>
                  </li>
                ),
              )}
            </ul>
            <p className="m-0 mt-auto text-[12.5px] text-muted-foreground">
              The other <span className="tabular-nums">{Math.max(0, paramCount - 7)}</span> settings keep
              their defaults, and each states why.
            </p>
          </CardBody>
        </Card>
      </button>

      <button
        type="button"
        onClick={() => onSelect("developer")}
        aria-pressed={chosen === "developer"}
        className={`rounded-card text-left transition-[box-shadow,transform] duration-100 ${
          chosen === "developer"
            ? "ring-[1.5px] ring-[var(--mc-gold)]"
            : "ring-1 ring-border hover:ring-muted-foreground"
        }`}
      >
        <Card className="h-full">
          <CardHeader>
            <CardTitle>Developer install</CardTitle>
            <Pill tone="neutral" className="ml-auto">
              every setting
            </Pill>
          </CardHeader>
          <CardBody className="gap-3">
            <p className="m-0 text-[13px] leading-[1.55] text-muted-foreground">
              Every setting, for a deployment you intend to run and maintain.
            </p>
            <ul className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13px]">
              {[
                `All ${paramCount} settings, searchable by name or by effect`,
                "What each one requires, and what it cannot be combined with",
                "The resolved env file before anything is written",
                "The compose command apply will run",
              ].map((item) => (
                <li key={item} className="flex items-start gap-2">
                  <Icon name="sliders" size={14} className="mt-0.5 shrink-0 text-muted-foreground" />
                  <span>{item}</span>
                </li>
              ))}
            </ul>
            <p className="m-0 mt-auto text-[12.5px] text-muted-foreground">
              Built for someone who will run the database, the backups and the upgrades.
            </p>
          </CardBody>
        </Card>
      </button>
    </div>
  );
}

/* ------------------------------------------------------------- 2. machine */

export function MachineStep({
  state,
  onUse,
  onDeclare,
  declared,
  onContinue,
}: {
  state: State;
  onUse: (profile: string) => void;
  onDeclare: (value: string) => void;
  declared: string;
  onContinue: () => void;
}) {
  const recommended = state.profiles.find((profile) => profile.name === state.recommendation.profile);
  const fitting = state.profiles.filter((profile) => profile.fits);

  return (
    <div className="flex max-w-[820px] flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-[14px]">Measured just now</CardTitle>
          <Button
            size="sm"
            variant="outline"
            className="ml-auto"
            onClick={() => window.location.reload()}
            aria-label="Measure this machine again"
          >
            <Icon name="refresh" size={14} /> Re-measure
          </Button>
        </CardHeader>
        <CardBody className="grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-3 lg:grid-cols-5">
          <Fact label="cores" value={`${state.host.cores}`} />
          <Fact
            label="memory"
            value={state.host.ram_mb === null ? "not published" : formatMb(state.host.ram_mb)}
            muted={state.host.ram_mb === null}
          />
          <Fact
            label="free disk"
            value={state.host.free_disk_gb === null ? "not readable" : `${state.host.free_disk_gb} GB`}
            muted={state.host.free_disk_gb === null}
          />
          <Fact label="docker" value={state.host.docker ?? "no answer"} muted={state.host.docker === null} />
          <Fact label="gpu" value={state.host.gpu ?? "none"} muted={state.host.gpu === null} />
        </CardBody>
      </Card>

      {recommended ? (
        <Card>
          <CardHeader>
            <Icon name="spark" size={16} className="text-[var(--mc-gold-text)]" />
            <CardTitle className="text-[14px]">
              Recommended: <span className="font-mono text-[13px]">{recommended.name}</span>
            </CardTitle>
          </CardHeader>
          <CardBody>
            <ul className="m-0 flex list-none flex-col gap-1.5 p-0 text-[13px] leading-[1.5]">
              {state.recommendation.why
                .filter((line) => line.includes(recommended.name))
                .map((line) => (
                  <li key={line} className="flex items-start gap-2">
                    <span aria-hidden="true" className="mt-[7px] size-1 shrink-0 rounded-full bg-[var(--mc-gold)]" />
                    <span>{line}</span>
                  </li>
                ))}
            </ul>
            {state.recommendation.why.filter((line) => !line.includes(recommended.name)).length > 0 ? (
              <p className="m-0 text-[12.5px] text-muted-foreground">
                Also fits:{" "}
                {state.recommendation.why
                  .filter((line) => !line.includes(recommended.name))
                  .map((line) => line.split(" fits:")[0])
                  .join(" · ")}
              </p>
            ) : null}
            <p className="m-0 mt-1 text-[12.5px] text-muted-foreground">{state.embedding}</p>
          </CardBody>
        </Card>
      ) : null}

      {fitting.length === 0 ? (
        <Banner
          icon={<Icon name="alert" size={16} />}
          action={
            <Button size="sm" variant="strip" onClick={onContinue}>
              Continue anyway
            </Button>
          }
        >
          <strong className="block font-semibold">No shipped shape fits this machine as it stands.</strong>
          <span className="block">
            {state.profiles
              .filter((profile) => !profile.fits)
              .map((profile) => `${profile.name}: ${profile.missing.join("; ")}`)
              .join(" · ")}
          </span>
        </Banner>
      ) : null}

      {state.host.ram_mb === null ? (
        <Card>
          <CardBody>
            <Field
              label="This platform publishes no memory reading"
              hint="Declare it and the shapes are scored on it. This is the same value MARULA_HOST_MEMORY_MB carries."
            >
              <Input
                inputMode="numeric"
                placeholder="16384"
                value={declared}
                onChange={(event) => onDeclare(event.target.value)}
              />
            </Field>
          </CardBody>
        </Card>
      ) : null}

      <div className="flex items-center justify-end gap-2.5">
        <span className="mr-auto flex items-center gap-1 text-[12px] text-muted-foreground">
          <Kbd>⌘</Kbd>
          <Kbd>↵</Kbd> continues
        </span>
        <Button variant="primary" onClick={onContinue}>
          Continue
        </Button>
      </div>
    </div>
  );
}

function Fact({ label, value, muted }: { label: string; value: string; muted?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col">
      <span className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground">{label}</span>
      <span className={`truncate font-mono text-[13px] tabular-nums ${muted ? "text-muted-foreground" : ""}`}>
        {value}
      </span>
    </div>
  );
}

/* --------------------------------------------------------------- 3. shape */

export function ShapeStep({
  state,
  chosen,
  onChoose,
}: {
  state: State;
  chosen: string;
  onChoose: (profile: string) => void;
}) {
  return (
    <div className="grid max-w-[900px] grid-cols-1 gap-4 md:grid-cols-2">
      {state.profiles.map((profile) => (
        <ShapeCard
          key={profile.name}
          profile={profile}
          host={state.host}
          selected={chosen === profile.name}
          recommended={state.recommendation.profile === profile.name}
          onChoose={() => onChoose(profile.name)}
        />
      ))}
    </div>
  );
}

function ShapeCard({
  profile,
  host,
  selected,
  recommended,
  onChoose,
}: {
  profile: Profile;
  host: HostFacts;
  selected: boolean;
  recommended: boolean;
  onChoose: () => void;
}) {
  const rows = floorRows(profile, host);
  const oneLine = shapeLine(profile);
  return (
    <Card
      role="radio"
      aria-checked={selected}
      aria-disabled={!profile.fits}
      tabIndex={profile.fits ? 0 : -1}
      onClick={() => profile.fits && onChoose()}
      onKeyDown={(event) => {
        if ((event.key === "Enter" || event.key === " ") && profile.fits) {
          event.preventDefault();
          onChoose();
        }
      }}
      className={`flex cursor-pointer flex-col transition-colors ${
        selected ? "border-[var(--mc-gold)] ring-1 ring-[var(--mc-gold)]" : ""
      } ${profile.fits ? "" : "cursor-not-allowed opacity-70"}`}
    >
      <CardHeader>
        <CardTitle>{profile.title}</CardTitle>
        {recommended ? <Pill tone="gold" className="ml-auto">recommended</Pill> : null}
        {!recommended && !profile.fits ? <Pill tone="attention" className="ml-auto">too small</Pill> : null}
      </CardHeader>
      <CardBody className="gap-2.5">
        <code className="font-mono text-[11px] text-muted-foreground">{profile.name}</code>
        <p className="m-0 text-[13px] leading-[1.5]">{oneLine}</p>

        <div className="flex flex-wrap gap-1.5">
          {rows.map((row) => (
            <Pill
              key={row.label}
              tone={row.ok ? "success" : "attention"}
              dot
              title={`needs ${row.need}, this machine has ${row.have}`}
            >
              {row.label} {row.need}
            </Pill>
          ))}
        </div>

        <p className="m-0 font-mono text-[11px] text-muted-foreground">{servicesLine(profile)}</p>

        <details className="group text-[12px]">
          <summary className="flex cursor-pointer items-center gap-1 text-[var(--mc-gold-text)] marker:content-none">
            <Icon name="chev" size={12} />
            <span className="font-semibold">What this shape means</span>
          </summary>
          <p className="m-0 mt-1.5 leading-[1.55] text-muted-foreground">{profile.description}</p>
        </details>

        {profile.sets.length > 0 ? (
          <details className="group text-[12px]">
            <summary className="flex cursor-pointer items-center gap-1 text-[var(--mc-gold-text)] marker:content-none">
              <Icon name="chev" size={12} />
              <span className="font-semibold">{profile.sets.length} values set for you</span>
            </summary>
            <dl className="m-0 mt-1.5 grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-3 gap-y-1">
              {profile.sets.slice(0, 10).map((set) => (
                <div key={set.name} className="contents">
                  <dt className="truncate font-mono text-[10.5px] text-muted-foreground" title={set.name}>
                    {set.name}
                  </dt>
                  <dd className="m-0 truncate font-mono text-[10.5px]" title={set.value}>
                    {set.value}
                  </dd>
                </div>
              ))}
            </dl>
          </details>
        ) : null}

        {!profile.fits ? (
          <p className="m-0 border-t border-border pt-2 text-[12px] text-[var(--mc-attn-fg)]">
            {profile.missing.join("; ")}
          </p>
        ) : null}
      </CardBody>
    </Card>
  );
}

/* ------------------------------------------------------------ 4. settings */

export function SettingsStep({
  state,
  profile,
  overrides,
  onSet,
  mode,
  shape,
  onShape,
}: {
  state: State;
  profile: string;
  overrides: Record<string, string>;
  onSet: (name: string, value: string) => void;
  mode: PathMode;
  shape: ShapeChoice;
  onShape: (shape: ShapeChoice) => void;
}) {
  return (
    <div className="flex flex-col gap-7">
      <DeploymentSection shape={shape} onShape={onShape} />
      {mode === "recommended" ? (
        <RecommendedSettings state={state} profile={profile} overrides={overrides} onSet={onSet} />
      ) : (
        <DeveloperSettings state={state} profile={profile} overrides={overrides} onSet={onSet} />
      )}
    </div>
  );
}

export interface ShapeChoice {
  images: "build" | "registry";
  restart: "unless-stopped" | "always" | "no";
}

const RESTART_NOTE: Record<ShapeChoice["restart"], string> = {
  "unless-stopped":
    "Comes back after a crash and after the Docker daemon restarts. A deliberate stop stays stopped. It only starts at boot if Docker itself does.",
  always: "Comes back even after a deliberate stop. That surprises people who ran compose stop.",
  no: "Stays down when it exits. Right for a box you are working on. Wrong for one that serves a team.",
};

/** The two decisions that are not environment variables: where the images come
 *  from, and what Docker does with an exit. */
function DeploymentSection({
  shape,
  onShape,
}: {
  shape: ShapeChoice;
  onShape: (shape: ShapeChoice) => void;
}) {
  return (
    <Section title="Deployment">
      <div className="flex max-w-[720px] flex-col gap-5">
        <div className="flex flex-col gap-2">
          <p className="m-0 text-[13px] font-medium">Application images</p>
          <Segmented
            type="single"
            value={shape.images}
            onValueChange={(value) => value && onShape({ ...shape, images: value as ShapeChoice["images"] })}
            aria-label="Image source"
          >
            <SegmentedItem value="build">Build on this machine</SegmentedItem>
            <SegmentedItem value="registry">Pull the published images</SegmentedItem>
          </Segmented>
          <p className="m-0 text-[11.5px] leading-[1.5] text-muted-foreground">
            {shape.images === "build"
              ? "Compiles the four services here, which needs the source tree plus Rust and Node. That makes it the developer path, not the server one."
              : "Pulls the exact images the release published and skips the build. It needs the digests from that release; the terminal takes them with --image-digest."}
          </p>
        </div>

        <div className="flex flex-col gap-2">
          <p className="m-0 text-[13px] font-medium">If a service stops</p>
          <Segmented
            type="single"
            value={shape.restart}
            onValueChange={(value) => value && onShape({ ...shape, restart: value as ShapeChoice["restart"] })}
            aria-label="Restart policy"
          >
            <SegmentedItem value="unless-stopped">Restart unless stopped</SegmentedItem>
            <SegmentedItem value="always">Always restart</SegmentedItem>
            <SegmentedItem value="no">Leave it stopped</SegmentedItem>
          </Segmented>
          <p className="m-0 text-[11.5px] leading-[1.5] text-muted-foreground">
            {RESTART_NOTE[shape.restart]}
          </p>
        </div>
      </div>
    </Section>
  );
}

function RecommendedSettings({
  state,
  profile,
  overrides,
  onSet,
}: {
  state: State;
  profile: string;
  overrides: Record<string, string>;
  onSet: (name: string, value: string) => void;
}) {
  const fields = curatedFields(state);
  const groups = useMemo(() => {
    const byGroup = new Map<string, typeof fields>();
    for (const field of fields) {
      const list = byGroup.get(field.group) ?? [];
      list.push(field);
      byGroup.set(field.group, list);
    }
    return [...byGroup.entries()];
  }, [fields]);

  return (
    <div className="flex max-w-[720px] flex-col gap-6">
      {groups.map(([group, list]) => (
        <Section key={group} title={group}>
          <div className="flex flex-col gap-4">
            {list.map((field) => {
              const param = state.params.find((candidate) => candidate.name === field.name);
              if (!param) return null;
              const current = effectiveValue(param.name, state, profile, overrides);
              const overridden = Object.hasOwn(overrides, param.name);
              return (
                <div key={param.name} className="flex flex-col gap-1">
                  <Field
                    label={field.label}
                    hint={overridden ? param.consequence : field.hint ?? param.description}
                  >
                    <ParamControl
                      param={param}
                      value={current.value}
                      overridden={overridden}
                      onSet={onSet}
                    />
                  </Field>
                  {!overridden && param.default_rationale ? (
                    <p className="m-0 text-[11.5px] leading-[1.5] text-muted-foreground/80">
                      Default <span className="font-mono">{param.default}</span> — {param.default_rationale}
                    </p>
                  ) : null}
                </div>
              );
            })}
          </div>
        </Section>
      ))}
      <Callout icon={<Icon name="info" size={16} className="mt-0.5 shrink-0" />}>
        Everything else keeps its default. Switch to the developer install at any point to see all{" "}
        <span className="tabular-nums">{state.param_count}</span> parameters and the reason behind each one.
      </Callout>
    </div>
  );
}

function DeveloperSettings({
  state,
  profile,
  overrides,
  onSet,
}: {
  state: State;
  profile: string;
  overrides: Record<string, string>;
  onSet: (name: string, value: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<"all" | "yours">("all");
  const results = useMemo(() => {
    const searched = searchParams(state.params, query);
    return filter === "yours"
      ? searched.filter((param) => Object.hasOwn(overrides, param.name))
      : searched;
  }, [state.params, query, filter, overrides]);
  const groups = useMemo(() => paramsByGroup(results), [results]);

  return (
    <div className="flex max-w-[820px] flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2.5">
        <label className="flex h-ctl min-w-0 flex-[1_1_320px] items-center gap-2 rounded-ctl border border-border bg-card px-3 text-muted-foreground">
          <Icon name="search" size={16} />
          <span className="sr-only">Search parameters</span>
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Name, group, or a word from what it changes"
            className="min-w-0 flex-1 border-0 bg-transparent py-1 text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          <Kbd>⌘K</Kbd>
        </label>
        <Segmented
          type="single"
          value={filter}
          onValueChange={(value) => value && setFilter(value as "all" | "yours")}
          aria-label="Filter settings"
        >
          <SegmentedItem value="all">all {state.param_count}</SegmentedItem>
          <SegmentedItem value="yours">yours {Object.keys(overrides).length}</SegmentedItem>
        </Segmented>
      </div>

      {groups.length === 0 ? (
        <Callout>No setting matches that. Try a name, a group, or a word from its consequence.</Callout>
      ) : null}

      {groups.map((group) => (
        <Section key={group.group} title={group.title}>
          <div className="overflow-hidden rounded-card border border-border bg-card">
            {group.params.map((param) => (
              <ParamRow
                key={param.name}
                param={param}
                state={state}
                profile={profile}
                overrides={overrides}
                onSet={onSet}
              />
            ))}
          </div>
        </Section>
      ))}
    </div>
  );
}

function ParamRow({
  param,
  state,
  profile,
  overrides,
  onSet,
}: {
  param: Param;
  state: State;
  profile: string;
  overrides: Record<string, string>;
  onSet: (name: string, value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const current = effectiveValue(param.name, state, profile, overrides);
  const overridden = Object.hasOwn(overrides, param.name);
  const warnings = warningsFor(param, state, profile, overrides);

  return (
    <div className="border-b border-border last:border-b-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="grid w-full grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 px-3.5 py-2.5 text-left hover:bg-muted"
      >
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium">{humanLabel(param.name)}</span>
          <code className="block truncate font-mono text-[10.5px] text-muted-foreground">{param.name}</code>
        </span>
        <code className="max-w-[220px] truncate font-mono text-[12px] tabular-nums">
          {current.value ?? "—"}
        </code>
        <span className="flex items-center gap-1.5">
          {param.required ? <Pill tone="attention">required</Pill> : null}
          {overridden ? <Pill tone="gold">yours</Pill> : null}
          <Icon name={open ? "up" : "down"} size={14} className="text-muted-foreground" />
        </span>
      </button>

      {open ? (
        <div className="flex flex-col gap-2.5 px-3.5 pb-3.5">
          <p className="m-0 text-[12.5px] text-muted-foreground">{param.description}</p>
          <ParamControl param={param} value={current.value} overridden={overridden} onSet={onSet} />
          <p className="m-0 text-[12px] leading-[1.5]">
            <span className="mr-1.5 text-[10px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
              what changes
            </span>
            {param.consequence}
          </p>
          {param.default_rationale ? (
            <p className="m-0 text-[12px] leading-[1.5]">
              <span className="mr-1.5 text-[10px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
                why the default
              </span>
              {param.default_rationale}
            </p>
          ) : null}
          {param.requires.length > 0 ? (
            <p className="m-0 font-mono text-[11px] text-muted-foreground">needs {param.requires.join(", ")}</p>
          ) : null}
          {param.conflicts.length > 0 ? (
            <p className="m-0 font-mono text-[11px] text-muted-foreground">
              cannot be set with {param.conflicts.join(", ")}
            </p>
          ) : null}
          {warnings.map((warning) => (
            <p key={warning.text} className="m-0 text-[12px] text-[var(--mc-attn-fg)]">
              {warning.text}
            </p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** The control a lever gets: a switch for a boolean, a select for a closed set,
 *  a text field for everything else. The set comes from the descriptor. */
/** A closed-set token as a person reads it: `ollama` is Ollama, `s3` is S3. */
function optionLabel(value: string): string {
  if (/^[a-z]{1,3}\d?$/i.test(value) && value.length <= 3) return value.toUpperCase();
  return value.charAt(0).toUpperCase() + value.slice(1);
}

function ParamControl({
  param,
  value,
  overridden,
  onSet,
}: {
  param: Param;
  value: string | null;
  overridden: boolean;
  onSet: (name: string, value: string) => void;
}) {
  if (param.values.length > 0) {
    const current = value ?? "";
    const known = param.values.includes(current);
    return (
      <div className="flex items-center gap-2">
        <Select
          value={known ? current : ""}
          onChange={(event) => onSet(param.name, event.target.value)}
          aria-label={param.name}
        >
          {known ? null : <option value="">{current ? `${current} (current)` : "choose…"}</option>}
          {param.values.map((option) => (
            <option key={option} value={option}>
              {optionLabel(option)}
            </option>
          ))}
        </Select>
        {overridden ? (
          <Button size="sm" variant="outline" onClick={() => onSet(param.name, "")}>
            reset
          </Button>
        ) : null}
      </div>
    );
  }

  if (param.kind === "boolean") {
    const checked = (value ?? "").toLowerCase() === "true" || value === "1";
    return (
      <label className="flex items-center gap-2.5 text-[13px]">
        <Switch checked={checked} onCheckedChange={(next) => onSet(param.name, next ? "true" : "false")} />
        {checked ? "on" : "off"}
      </label>
    );
  }
  return (
    <div className="flex items-center gap-2">
      <Input
        value={param.secret && !overridden ? "" : value ?? ""}
        placeholder={param.secret ? "unchanged unless you type here" : param.default ?? "not set"}
        onChange={(event) => onSet(param.name, event.target.value)}
        spellCheck={false}
        autoComplete="off"
      />
      {overridden ? (
        <Button size="sm" variant="outline" onClick={() => onSet(param.name, "")}>
          reset
        </Button>
      ) : null}
    </div>
  );
}

/* -------------------------------------------------------------- 5. review */

export function ReviewStep({
  state,
  profile,
  plan,
  planError,
  apply,
  doctor,
  doctorError,
  onApply,
  onDoctor,
  onOpen,
  onMode,
  mode,
}: {
  state: State;
  profile: string;
  plan: Plan | null;
  planError: string | null;
  apply: ApplyState | null;
  doctor: Doctor | null;
  doctorError: string | null;
  onApply: () => void;
  onDoctor: () => void;
  onOpen: () => void;
  onMode: (mode: PathMode) => void;
  mode: PathMode;
}) {
  const [terminal, setTerminal] = useState(false);
  const chosen = state.profiles.find((candidate) => candidate.name === profile);
  const counts = sourceCounts(plan?.config ?? state.env);
  const rows = plan?.config ?? [];
  const progress = plan && plan.steps.length > 0 && apply ? Math.round((apply.done.length / plan.steps.length) * 100) : 0;

  return (
    <div className="flex max-w-[880px] flex-col gap-5">
      <Card>
        <CardBody className="grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-4">
          <SummaryItem label="shape" value={chosen?.title ?? profile} />
          <SummaryItem label="processes" value={`${chosen?.services.length ?? 0}`} />
          <SummaryItem label="values you set" value={`${counts.you}`} />
          <SummaryItem label="generated" value={`${counts.generated}`} />
        </CardBody>
      </Card>

      {planError ? (
        <Banner icon={<Icon name="alert" size={16} />}>
          <strong className="block font-semibold">This plan cannot run as it stands</strong>
          <span className="block font-mono text-[12px]">{planError}</span>
        </Banner>
      ) : null}

      {plan ? <PlanList steps={plan.steps} /> : null}

      {rows.length > 0 ? (
        <Section
          title="What it writes"
          hint="Grouped by what the value configures. Hover a row for the exact setting name."
          action={
            <Button size="sm" variant="outline" onClick={() => onMode(mode === "recommended" ? "developer" : "recommended")}>
              {mode === "recommended" ? "Edit all parameters" : "Back to recommended"}
            </Button>
          }
        >
          <div className="value-scroll pr-1">
            <ValueTable rows={rows} />
          </div>
        </Section>
      ) : null}

      {plan && plan.manual.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle className="text-[14px]">Still yours to do</CardTitle>
          </CardHeader>
          <CardBody className="gap-1.5">
            {plan.manual.map((item) => (
              <p key={item} className="m-0 flex items-start gap-2 text-[13px]">
                <Icon name="ext" size={14} className="mt-0.5 shrink-0 text-muted-foreground" />
                {item}
              </p>
            ))}
          </CardBody>
        </Card>
      ) : null}

      {plan ? (
        <div>
          <Button size="sm" variant="outline" onClick={() => setTerminal((open) => !open)}>
            {terminal ? "Hide" : "Show"} the plan as text
          </Button>
          {terminal ? (
            <pre className="mt-2.5 max-h-[300px] overflow-auto rounded-card bg-[var(--mc-bar)] p-3.5 font-mono text-[11px] leading-[1.6] text-[var(--mc-cream)]">
              {plan.raw}
            </pre>
          ) : null}
        </div>
      ) : null}

      {apply === null ? (
        <div className="flex items-center justify-end gap-2.5 border-t border-border pt-4">
          <span className="mr-auto text-[12px] text-muted-foreground">
            The first apply builds images, so expect minutes.
          </span>
          <Button variant="primary" onClick={onApply} disabled={!plan}>
            Apply and start
          </Button>
        </div>
      ) : (
        <ApplyPanel apply={apply} progress={progress} onApply={onApply} />
      )}

      {doctor ? <DoctorPanel doctor={doctor} onDoctor={onDoctor} onOpen={onOpen} /> : null}
      {doctorError ? (
        <Banner icon={<Icon name="alert" size={16} />} action={<Button size="sm" variant="strip" onClick={onDoctor}>Run again</Button>}>
          <span className="block font-mono text-[12px]">{doctorError}</span>
        </Banner>
      ) : null}
    </div>
  );
}

function SummaryItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-col">
      <span className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground">{label}</span>
      <span className="truncate text-[13px] font-medium">{value}</span>
    </div>
  );
}

function ApplyPanel({
  apply,
  progress,
  onApply,
}: {
  apply: ApplyState;
  progress: number;
  onApply: () => void;
}) {
  if (apply.running) {
    return (
      <Card aria-live="polite">
        <CardHeader>
          <CardTitle className="text-[14px]">Applying</CardTitle>
          <span className="ml-auto text-[12px] tabular-nums text-muted-foreground">{progress}%</span>
        </CardHeader>
        <CardBody>
          <div className="h-2 overflow-hidden rounded-pill bg-muted">
            <div className="h-full bg-[var(--mc-green)] transition-[width] duration-100" style={{ width: `${progress}%` }} />
          </div>
          {apply.done.length > 0 ? (
            <ol className="m-0 flex list-none flex-col gap-1 p-0">
              {apply.done.map((line) => (
                <li key={line} className="flex items-start gap-2 font-mono text-[11px]">
                  <Icon name="check" size={13} className="mt-0.5 shrink-0 text-[var(--mc-green)]" />
                  <span className="break-words">{line}</span>
                </li>
              ))}
            </ol>
          ) : null}
          <p className="m-0 text-[12px] text-muted-foreground">
            Waiting for the stack's healthchecks. This returns when they pass, not when the command exits.
          </p>
        </CardBody>
      </Card>
    );
  }

  if (apply.failed) {
    return (
      <Banner
        icon={<Icon name="alert" size={16} />}
        action={<Button size="sm" variant="strip" onClick={onApply}>Apply again</Button>}
      >
        <strong className="block font-semibold">The apply stopped</strong>
        <span className="block font-mono text-[12px]">{apply.failed}</span>
        <span className="block">The env file is reused, so a retry rotates no value.</span>
      </Banner>
    );
  }

  return (
    <Callout icon={<Icon name="check" size={16} className="mt-0.5 shrink-0 text-[var(--mc-green)]" />}>
      Applied. {apply.done.length} steps ran and the stack answered.
    </Callout>
  );
}

function DoctorPanel({
  doctor,
  onDoctor,
  onOpen,
}: {
  doctor: Doctor;
  onDoctor: () => void;
  onOpen: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-[14px]">Doctor</CardTitle>
        <Pill tone={doctor.ok ? "success" : "attention"} className="ml-auto" dot>
          {doctor.ok ? "healthy" : "not healthy"}
        </Pill>
      </CardHeader>
      <CardBody className="gap-1.5">
        {doctor.checks.map((check) => (
          <CheckRow key={check.name} check={check} />
        ))}
        {doctor.services.length > 0 ? (
          <p className="m-0 mt-1 font-mono text-[11px] text-muted-foreground">
            running: {doctor.services.join(", ")}
          </p>
        ) : null}
        {doctor.error ? <p className="m-0 font-mono text-[11px]">{doctor.error}</p> : null}
        <div className="mt-1.5 flex justify-end gap-2">
          <Button size="sm" variant="outline" onClick={onDoctor}>
            Check again
          </Button>
          <Button size="sm" variant="primary" onClick={onOpen} disabled={!doctor.ok}>
            Open Marula
          </Button>
        </div>
      </CardBody>
    </Card>
  );
}

function CheckRow({ check }: { check: Check }) {
  return (
    <div className="grid grid-cols-[16px_140px_minmax(0,1fr)] items-baseline gap-2 text-[12px]">
      <Icon
        name={check.ok ? "check" : "alert"}
        size={13}
        className={check.ok ? "text-[var(--mc-green)]" : "text-[var(--mc-attn-fg)]"}
      />
      <span className="truncate font-mono text-[11px]">{check.name}</span>
      <span className="text-muted-foreground">{check.detail}</span>
    </div>
  );
}
