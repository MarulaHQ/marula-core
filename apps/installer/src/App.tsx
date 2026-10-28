/**
 * The installer's shell: a rail, five decisions, and one summary panel.
 *
 * Both hosts mount this component (`main.tsx`), so the native app and the
 * loopback fallback are the same screens. The rail is the only navigation, and
 * ⌘K goes straight to the developer surface from anywhere.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Card, CardBody, CardHeader, CardTitle, Icon, Kbd, Pill, Wordmark } from "@marula-core/ui/kit";

import { errorMessage, transport } from "./api";
import {
  STEPS,
  canLeave,
  effectiveValue,
  humanLabel,
  sourceCounts,
  startingProfile,
  type PathMode,
} from "./lib/model";
import { MachineStep, PathStep, ReviewStep, SettingsStep, ShapeStep } from "./steps";
import type { ApplyState, Doctor, Plan, State } from "./types";

type ThemeChoice = "system" | "light" | "dark";

const THEME_KEY = "marula.installer.theme";

export function App() {
  const [state, setState] = useState<State | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [step, setStep] = useState(0);
  const [mode, setMode] = useState<PathMode>("recommended");
  const [profile, setProfile] = useState("");
  const [overrides, setOverrides] = useState<Record<string, string>>({});
  const [declared, setDeclared] = useState("");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [apply, setApply] = useState<ApplyState | null>(null);
  const [doctor, setDoctor] = useState<Doctor | null>(null);
  const [doctorError, setDoctorError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [theme, setTheme] = useState<ThemeChoice>(() => {
    const saved = typeof localStorage === "undefined" ? null : localStorage.getItem(THEME_KEY);
    return saved === "light" || saved === "dark" || saved === "system" ? saved : "system";
  });

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const next = await transport.state();
      setState(next);
      setProfile((current) => current || startingProfile(next));
    } catch (error) {
      setLoadError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!state || step !== 4) return;
    let cancelled = false;
    void (async () => {
      try {
        const next = await transport.plan({ profile, overrides });
        if (!cancelled) {
          setPlan(next);
          setPlanError(null);
        }
      } catch (error) {
        if (!cancelled) {
          setPlan(null);
          setPlanError(errorMessage(error));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [state, step, profile, overrides]);

  const runDoctor = useCallback(async () => {
    setDoctorError(null);
    try {
      setDoctor(await transport.doctor(profile));
    } catch (error) {
      setDoctor(null);
      setDoctorError(errorMessage(error));
    }
  }, [profile]);

  useEffect(() => {
    if (!apply?.running) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const next = await transport.applyState();
        if (cancelled) return;
        setApply(next);
        if (next.running) window.setTimeout(poll, 900);
        else if (next.failed === null) await runDoctor();
      } catch (error) {
        if (cancelled) return;
        setApply((current) =>
          current
            ? { ...current, running: false, failed: errorMessage(error) }
            : { running: false, done: [], failed: errorMessage(error), manual: [] },
        );
      }
    };
    const first = window.setTimeout(poll, 700);
    return () => {
      cancelled = true;
      window.clearTimeout(first);
    };
  }, [apply?.running, runDoctor]);

  useEffect(() => {
    if (typeof localStorage !== "undefined") localStorage.setItem(THEME_KEY, theme);
    applyTheme(theme);
    if (theme !== "system" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = () => applyTheme("system");
    query.addEventListener("change", listener);
    return () => query.removeEventListener("change", listener);
  }, [theme]);

  const chooseProfile = useCallback((next: string) => {
    setProfile(next);
    setApply(null);
    setDoctor(null);
  }, []);

  const setOverride = useCallback((name: string, value: string) => {
    setOverrides((current) => {
      const copy = { ...current };
      if (value === "") delete copy[name];
      else copy[name] = value;
      return copy;
    });
    setApply(null);
    setDoctor(null);
  }, []);

  const applyPlan = useCallback(async () => {
    setBusy(true);
    try {
      await transport.apply({ profile, overrides });
      setApply({ running: true, done: [], failed: null, manual: [] });
    } catch (error) {
      setApply({ running: false, done: [], failed: errorMessage(error), manual: [] });
    } finally {
      setBusy(false);
    }
  }, [profile, overrides]);

  const goTo = useCallback(
    (next: number) => {
      if (!state) return;
      if (next > step && !canLeave(step, state, profile)) return;
      setStep(Math.max(0, Math.min(STEPS.length - 1, next)));
    },
    [state, step, profile],
  );

  const advance = useCallback(() => {
    if (step === STEPS.length - 1) {
      void applyPlan();
      return;
    }
    if (step === 0) {
      goTo(1);
      return;
    }
    goTo(step + 1);
  }, [step, goTo, applyPlan]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const meta = event.metaKey || event.ctrlKey;
      if (meta && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setMode("developer");
        setStep(3);
        return;
      }
      if (meta && event.key === "Enter") {
        event.preventDefault();
        advance();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [advance]);

  if (loadError) {
    return (
      <main className="mx-auto flex max-w-[640px] flex-col gap-4 p-14">
        <Wordmark size={26} />
        <Card>
          <CardHeader>
            <Icon name="alert" size={16} className="text-[var(--mc-attn-fg)]" />
            <CardTitle className="text-[14px]">The installer could not read this machine</CardTitle>
          </CardHeader>
          <CardBody>
            <p className="m-0 font-mono text-[12px]">{loadError}</p>
            <p className="m-0 text-[12px] text-muted-foreground">
              In the browser fallback, open the exact link the terminal printed. It carries a one-time
              token, and the cookie it sets is what this page needs.
            </p>
            <button
              type="button"
              onClick={() => void load()}
              className="mt-1 self-start rounded-ctl border border-border px-3.5 py-2 text-[13px] font-semibold hover:bg-muted"
            >
              Try again
            </button>
          </CardBody>
        </Card>
      </main>
    );
  }

  if (!state) {
    return (
      <main className="mx-auto flex h-full max-w-[640px] items-center gap-3 p-14 text-muted-foreground">
        <Wordmark size={24} />
        <span>reading this machine…</span>
      </main>
    );
  }

  const meta = STEPS[step] ?? STEPS[0]!;

  return (
    <div className="grid h-screen grid-cols-1 grid-rows-[auto_minmax(0,1fr)] lg:grid-cols-[232px_minmax(0,1fr)] xl:grid-cols-[232px_minmax(0,1fr)_320px] xl:grid-rows-1">
      <nav
        aria-label="Install steps"
        className="flex flex-row items-center gap-3 border-b border-border bg-mc-side px-3 py-2.5 lg:flex-col lg:items-stretch lg:border-r lg:border-b-0 lg:px-3 lg:py-4"
      >
        <div className="hidden px-2 lg:block">
          <Wordmark size={24} />
          <p className="m-0 mt-0.5 pl-[38px] text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
            installer
          </p>
        </div>

        <ol className="m-0 flex list-none flex-row gap-1 overflow-x-auto p-0 lg:mt-5 lg:flex-col">
          {STEPS.map((item, index) => {
            const done = index < step;
            const current = index === step;
            return (
              <li key={item.id}>
                <button
                  type="button"
                  aria-current={current ? "step" : undefined}
                  disabled={index > step}
                  onClick={() => goTo(index)}
                  className={`flex h-9 w-full items-center gap-2 whitespace-nowrap rounded-ctl px-2.5 text-left text-[13px] transition-colors ${
                    current
                      ? "bg-card font-semibold text-foreground shadow-[inset_2px_0_0_var(--mc-gold)]"
                      : done
                        ? "text-muted-foreground hover:bg-muted"
                        : "text-muted-foreground/70"
                  }`}
                >
                  <span className="w-3.5 font-mono text-[10.5px] tabular-nums">
                    {done ? <Icon name="check" size={12} className="text-[var(--mc-green)]" /> : `0${index + 1}`}
                  </span>
                  {item.label}
                </button>
              </li>
            );
          })}
        </ol>

        <div className="ml-auto flex flex-col items-end gap-1.5 lg:mt-auto lg:items-start">
          <div className="flex overflow-hidden rounded-pill border border-border bg-card">
            {(["system", "light", "dark"] as const).map((choice) => (
              <button
                key={choice}
                type="button"
                aria-pressed={theme === choice}
                onClick={() => setTheme(choice)}
                className={`px-2.5 py-1 text-[10.5px] capitalize ${
                  theme === choice ? "bg-mc-act font-semibold text-mc-act-fg" : "text-muted-foreground"
                }`}
              >
                {choice}
              </button>
            ))}
          </div>
          <p className="m-0 hidden text-[10px] text-muted-foreground lg:block">
            {transport.kind === "native" ? "native app" : "loopback fallback"} · no telemetry
          </p>
        </div>
      </nav>

      <main className="overflow-y-auto px-6 py-6 lg:px-10 lg:py-8">
        <header className="mb-5 flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className="font-mono text-[10.5px] tracking-[0.12em] text-[var(--mc-gold-text)]">
              0{step + 1}
            </span>
            <span className="font-mono text-[10.5px] text-muted-foreground">/ 05</span>
            {step > 0 ? (
              <Pill tone={mode === "developer" ? "neutral" : "gold"} className="ml-1 text-[10.5px]">
                {mode === "developer" ? "developer install" : "recommended"}
              </Pill>
            ) : null}
          </div>
          <h1
            className={
              step === 0
                ? "m-0 font-display text-[34px] font-normal leading-[1.1] tracking-[-0.01em]"
                : "m-0 text-[22px] font-semibold leading-tight tracking-[-0.01em]"
            }
          >
            {step === 0
              ? "How would you like to install Marula?"
              : step === 1
                ? "This machine"
                : step === 2
                  ? "Choose a shape"
                  : step === 3
                    ? mode === "developer"
                      ? "Every parameter"
                      : "What only you can decide"
                    : "Review, then apply"}
          </h1>
          <p className="m-0 mt-1 max-w-[62ch] text-[13.5px] leading-[1.55] text-muted-foreground">
            {step === 0
              ? "Two ways in. Both run the same engine and write the same install; they differ in how much you decide."
              : step === 1
                ? "Measured the same way the product measures itself at boot, so the recommendation and the refusal are the same verdict."
                : step === 2
                  ? "One decision. Both shapes run the product; they differ in what the machine must give them."
                  : step === 3
                    ? mode === "developer"
                      ? "The whole catalogue, each lever with what it changes and what it needs. Nothing here is hidden."
                      : "Seven answers, and every one of them has a reason printed under it."
                    : "The plan as data: every step, the reason it exists, and the values it writes."}
          </p>
        </header>

        {step === 0 ? (
          <PathStep
            paramCount={state.param_count}
            onChoose={(next) => {
              setMode(next);
              setStep(1);
            }}
          />
        ) : null}

        {step === 1 ? (
          <MachineStep
            state={state}
            onUse={chooseProfile}
            onDeclare={setDeclared}
            declared={declared}
            onContinue={advance}
          />
        ) : null}

        {step === 2 ? (
          <>
            <ShapeStep state={state} chosen={profile} onChoose={chooseProfile} />
            <div className="mt-5 flex items-center justify-end gap-2.5 border-t border-border pt-4">
              <span className="mr-auto flex items-center gap-1 text-[12px] text-muted-foreground">
                {state.profiles.some((candidate) => candidate.name === profile && candidate.fits) ? (
                  <>
                    <Kbd>⌘</Kbd>
                    <Kbd>↵</Kbd> continues
                  </>
                ) : (
                  "pick a shape this machine can carry"
                )}
              </span>
              <button
                type="button"
                onClick={() => goTo(1)}
                className="rounded-ctl border border-border px-3.5 py-2 text-[13px] font-semibold hover:bg-muted"
              >
                Back
              </button>
              <button
                type="button"
                onClick={advance}
                disabled={!state.profiles.some((candidate) => candidate.name === profile && candidate.fits)}
                className="rounded-ctl bg-primary px-3.5 py-2 text-[13px] font-semibold text-primary-foreground disabled:bg-muted disabled:text-muted-foreground"
              >
                Continue
              </button>
            </div>
          </>
        ) : null}

        {step === 3 ? (
          <>
            <SettingsStep
              state={state}
              profile={profile}
              overrides={overrides}
              onSet={setOverride}
              mode={mode}
            />
            <div className="mt-6 flex items-center justify-end gap-2.5 border-t border-border pt-4">
              <span className="mr-auto flex items-center gap-1 text-[12px] text-muted-foreground">
                <Kbd>⌘</Kbd>
                <Kbd>K</Kbd> the whole catalogue
              </span>
              <button
                type="button"
                onClick={() => goTo(2)}
                className="rounded-ctl border border-border px-3.5 py-2 text-[13px] font-semibold hover:bg-muted"
              >
                Back
              </button>
              <button
                type="button"
                onClick={advance}
                className="rounded-ctl bg-primary px-3.5 py-2 text-[13px] font-semibold text-primary-foreground"
              >
                Continue
              </button>
            </div>
          </>
        ) : null}

        {step === 4 ? (
          <>
            <ReviewStep
              state={state}
              profile={profile}
              plan={plan}
              planError={planError}
              apply={apply}
              doctor={doctor}
              doctorError={doctorError}
              onApply={() => void applyPlan()}
              onDoctor={() => void runDoctor()}
              onOpen={() => void transport.open(productUrl(state, profile, overrides))}
              onMode={(next) => {
                setMode(next);
                setStep(3);
              }}
              mode={mode}
            />
            <div className="mt-5 flex items-center justify-end gap-2.5 border-t border-border pt-4">
              <button
                type="button"
                onClick={() => goTo(3)}
                disabled={busy}
                className="rounded-ctl border border-border px-3.5 py-2 text-[13px] font-semibold hover:bg-muted"
              >
                Back
              </button>
            </div>
          </>
        ) : null}
      </main>

      <aside className="hidden flex-col border-l border-border bg-card xl:flex" aria-label="This install">
        <CardHeader className="rounded-none border-b border-border">
          <CardTitle className="text-[12px] uppercase tracking-[0.1em] text-muted-foreground">
            This install
          </CardTitle>
        </CardHeader>
        <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-4">
          <Summary label="shape" value={state.profiles.find((p) => p.name === profile)?.title ?? "choosing"} />
          <Summary
            label="path"
            value={mode === "developer" ? "developer install" : "recommended setup"}
          />
          <Summary label="directory" value={state.dir} mono />
          <Summary label="levers" value={`${state.param_count}`} />
          {plan ? (
            <Summary
              label="generated values"
              value={`${sourceCounts(plan.config).generated} of ${plan.config.length}`}
            />
          ) : null}
          {state.status.services.length > 0 ? (
            <div className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground">running now</span>
              <div className="flex flex-wrap gap-1">
                {state.status.services.map((service) => (
                  <Pill key={service} tone="success" dot className="text-[10.5px]">
                    {service}
                  </Pill>
                ))}
              </div>
            </div>
          ) : null}

          {plan ? <KeyValues rows={plan.config} /> : null}
        </div>
        <div className="border-t border-border p-4">
          <button
            type="button"
            onClick={() => {
              setMode("developer");
              setStep(3);
            }}
            className="flex items-center gap-1.5 text-[12px] font-semibold text-[var(--mc-gold-text)]"
          >
            All {state.param_count} parameters <Kbd>⌘K</Kbd>
          </button>
          <p className="m-0 mt-1.5 text-[10.5px] leading-[1.5] text-muted-foreground">
            Nothing leaves this machine. No telemetry, and every step works offline once the images are
            pulled.
          </p>
        </div>
      </aside>
    </div>
  );
}

function Summary({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col">
      <span className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground">{label}</span>
      <span className={`truncate text-[12.5px] ${mono ? "font-mono text-[11px]" : "font-medium"}`} title={value}>
        {value}
      </span>
    </div>
  );
}

/** The three values a person will look for after the install. */
function KeyValues({ rows }: { rows: Plan["config"] }) {
  const wanted = ["WEB_BASE_URL", "MARULA_EMBEDDING_PROVIDER", "OBJECT_STORE"];
  const kept = rows.filter((row) => wanted.includes(row.name));
  if (kept.length === 0) return null;
  return (
    <div className="mt-1 flex flex-col gap-1.5 border-t border-border pt-3">
      {kept.map((row) => (
        <div key={row.name} className="flex min-w-0 flex-col">
          <span className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
            {humanLabel(row.name)}
          </span>
          <code className="truncate font-mono text-[11px]">{row.value}</code>
        </div>
      ))}
    </div>
  );
}

function productUrl(state: State, profile: string, overrides: Record<string, string>): string {
  const web = effectiveValue("WEB_BASE_URL", state, profile, overrides).value;
  return web && web.startsWith("http") ? web : "http://localhost:8080";
}

function applyTheme(choice: ThemeChoice) {
  const root = document.documentElement;
  if (choice === "system") {
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
    if (dark) root.dataset.theme = "dark";
    else delete root.dataset.theme;
    return;
  }
  if (choice === "dark") root.dataset.theme = "dark";
  else delete root.dataset.theme;
}
