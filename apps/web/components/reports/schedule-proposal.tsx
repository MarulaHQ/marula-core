"use client";
/**
 * ScheduleProposal — "describe a schedule" for the Schedules tab.
 *
 * Composition, taken from the three real scheduling surfaces this lane studied
 * and not from their styling:
 *
 * - **The frequency picker inside a small modal** (Intercom's report dialog,
 *   Sprout Social's email-report dialog, Front's delivery dialog): a cadence
 *   control, a time control, and one summary line that restates the whole
 *   schedule in a sentence before anything is saved.
 * - **The summary-before-save line** is the part worth keeping. Those dialogs
 *   never make a person read the controls back; they write the sentence.
 * - **Label/marker typography from the editorial captures** (Formafantasma's
 *   numbered section markers, Stripe's quiet paper): small labels above their
 *   value, generous vertical rhythm, one accent used only on the action.
 *
 * Tokens are the product's: Figtree at the surface's own sizes, `--m-*`
 * surfaces through the Tailwind bridge, 16px card radius, borders over shadows,
 * and the only motion is a 120ms opacity/translate entrance that
 * `prefers-reduced-motion` collapses.
 *
 * Non-purpose: it does not save. The one action fills the EXISTING create
 * form's cadence fields, and the existing `POST /api/v1/report-schedules` call
 * keeps its typed 422. It also does not compute the next run — that is
 * `core-domain::cadence` on the server, and a second implementation here would
 * be a second answer.
 */
import { useId, useState } from "react";
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@marula-core/ui";
import { Banner } from "@marula-core/ui/kit";
import {
  formatScheduleSentence,
  MONTH_DAYS,
  parseSchedulePhrase,
  proposalProblem,
  toCadenceFields,
  WEEKDAYS,
  type ScheduleCadenceFields,
  type SchedulePhraseResult,
  type ScheduleProposal,
} from "../../lib/schedule-phrase";

export interface ScheduleProposalProps {
  /**
   * The zone the surface can infer — the browser's own, or a row's stored one.
   * Null means nothing can be inferred, and a sentence without a zone is then
   * refused instead of read in some default.
   */
  defaultTimezone: string | null;
  /** Fills the existing create form's cadence fields and opens it for review. */
  onUse: (fields: ScheduleCadenceFields) => void;
}

export function ScheduleProposal({ defaultTimezone, onUse }: ScheduleProposalProps) {
  const uid = useId();
  const [text, setText] = useState("");
  const [outcome, setOutcome] = useState<SchedulePhraseResult | null>(null);
  const [draft, setDraft] = useState<ScheduleProposal | null>(null);

  const read = () => {
    const result = parseSchedulePhrase(text, { timezone: defaultTimezone });
    setOutcome(result);
    setDraft(result.ok ? result.proposal : null);
  };

  const edit = (patch: Partial<ScheduleProposal>) => {
    setDraft((current) => (current === null ? current : { ...current, ...patch }));
  };

  const problem = draft === null ? null : proposalProblem(draft);

  return (
    <Card aria-labelledby={`${uid}-title`}>
      <CardHeader>
        <h2 id={`${uid}-title`} className="m-0 text-base font-semibold">
          Describe a schedule
        </h2>
        <span className="flex-1" />
        <span className="text-xs text-muted-foreground">
          Read into fields you can edit before saving
        </span>
      </CardHeader>
      <CardContent className="gap-3">
        <div className="flex flex-wrap items-end gap-2">
          <div className="min-w-0 flex-1">
            <Label htmlFor={`${uid}-phrase`}>In your own words</Label>
            <Input
              id={`${uid}-phrase`}
              value={text}
              placeholder="every Monday at 09:00"
              aria-describedby={`${uid}-hint`}
              onChange={(e) => {
                setText(e.target.value);
                // A proposal describes the sentence it was read from. Once that
                // sentence changes, showing the old one would state something
                // nobody asked for, so it goes and the box is read again.
                setOutcome(null);
                setDraft(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  read();
                }
              }}
            />
            <FieldHint id={`${uid}-hint`}>
              For example “daily at 6pm”, “every Monday at 09:00”, or “monthly on the
              15th at 07:00, Europe/London”.
            </FieldHint>
          </div>
          <Button variant="secondary" onClick={read} disabled={text.trim() === ""}>
            Read it
          </Button>
        </div>

        {outcome !== null && !outcome.ok && (
          <Banner>
            <span className="flex flex-col gap-1">
              <span>{outcome.message}</span>
              <span className="text-xs">
                Nothing was filled in. Your sentence stays above, unchanged, until you edit
                it.
              </span>
            </span>
          </Banner>
        )}

        {draft !== null && (
          <div className="flex flex-col gap-3 rounded-ctl border border-border bg-muted/40 p-3 animate-in fade-in-0 slide-in-from-bottom-1 duration-100 motion-reduce:animate-none">
            <div className="flex flex-col gap-1">
              <span className="text-xs uppercase tracking-wide text-muted-foreground">
                This is the schedule
              </span>
              <p aria-live="polite" className="m-0 text-base font-semibold">
                {formatScheduleSentence(draft)}
              </p>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <Label htmlFor={`${uid}-cadence`}>Cadence</Label>
                <Select
                  value={draft.cadence}
                  onValueChange={(value) =>
                    edit({
                      cadence: value as ScheduleProposal["cadence"],
                      dayOfWeek: value === "weekly" ? (draft.dayOfWeek ?? 0) : null,
                      dayOfMonth: value === "monthly" ? (draft.dayOfMonth ?? 1) : null,
                    })
                  }
                >
                  <SelectTrigger id={`${uid}-cadence`} className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="daily">Daily</SelectItem>
                    <SelectItem value="weekly">Weekly</SelectItem>
                    <SelectItem value="monthly">Monthly</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div>
                <Label htmlFor={`${uid}-time`}>Time (HH:MM)</Label>
                <Input
                  id={`${uid}-time`}
                  value={draft.timeOfDay}
                  placeholder="09:00"
                  onChange={(e) => edit({ timeOfDay: e.target.value })}
                />
              </div>

              {draft.cadence === "weekly" && (
                <div>
                  <Label htmlFor={`${uid}-weekday`}>Weekday</Label>
                  <Select
                    value={String(draft.dayOfWeek ?? 0)}
                    onValueChange={(value) => edit({ dayOfWeek: Number(value) })}
                  >
                    <SelectTrigger id={`${uid}-weekday`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {WEEKDAYS.map((day, index) => (
                        <SelectItem key={day} value={String(index)}>
                          {day}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}

              {draft.cadence === "monthly" && (
                <div>
                  <Label htmlFor={`${uid}-monthday`}>Day of the month</Label>
                  <Select
                    value={String(draft.dayOfMonth ?? 1)}
                    onValueChange={(value) => edit({ dayOfMonth: Number(value) })}
                  >
                    <SelectTrigger id={`${uid}-monthday`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {MONTH_DAYS.map((day) => (
                        <SelectItem key={day} value={String(day)}>
                          {day}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}

              <div>
                <Label htmlFor={`${uid}-zone`}>Time zone</Label>
                <Input
                  id={`${uid}-zone`}
                  value={draft.timezone}
                  placeholder="Europe/London"
                  onChange={(e) => edit({ timezone: e.target.value })}
                />
                <FieldHint>The wall clock above is read in this zone.</FieldHint>
              </div>
            </div>

            {problem !== null && <Banner>{problem}</Banner>}

            <div className="flex justify-end">
              <Button
                onClick={() => {
                  if (draft === null || problem !== null) return;
                  onUse(toCadenceFields(draft));
                }}
                disabled={problem !== null}
              >
                Use this schedule
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
