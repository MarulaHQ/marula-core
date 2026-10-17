import { describe, expect, it } from "vitest";
import {
  compactValue,
  durationText,
  finishedPrefix,
  firstFailedIndex,
  isRetryable,
  prettyJson,
  relativeTime,
  runStatusMeta,
  shortId,
  spanText,
  stepOutcome,
  stepSummary,
  triggerLabel,
  type RunStepView,
} from "./run-labels";

function step(partial: Partial<RunStepView> & Pick<RunStepView, "step_index" | "status">): RunStepView {
  return {
    id: `00000000-0000-0000-0000-00000000000${partial.step_index}`,
    node_id: "ticket.create",
    node_label: "Create ticket",
    attempt: 1,
    input: {},
    started_at: "2026-10-16T10:00:00Z",
    ...partial,
  } as RunStepView;
}

describe("run status vocabulary", () => {
  it("names every status the server can send, with a kit tone", () => {
    expect(runStatusMeta("running")).toEqual({ label: "Running", tone: "progress" });
    expect(runStatusMeta("waiting")).toEqual({ label: "Waiting", tone: "waiting" });
    expect(runStatusMeta("succeeded")).toEqual({ label: "Succeeded", tone: "success" });
    // A failure is blush attention, never the red breach pair.
    expect(runStatusMeta("failed")).toEqual({ label: "Failed", tone: "attention" });
    expect(runStatusMeta("abandoned")).toEqual({ label: "Abandoned", tone: "neutral" });
  });

  it("offers a retry only where the server would allow one", () => {
    expect(isRetryable("failed")).toBe(true);
    expect(isRetryable("abandoned")).toBe(true);
    expect(isRetryable("running")).toBe(false);
    expect(isRetryable("waiting")).toBe(false);
    expect(isRetryable("succeeded")).toBe(false);
  });

  it("labels every trigger kind", () => {
    expect(triggerLabel("event")).toBe("From an event");
    expect(triggerLabel("operator_retry")).toBe("Operator retry");
  });
});

describe("step outcome", () => {
  it("separates a branch not taken from a step the run never reached", () => {
    const steps = [
      step({ step_index: 1, status: "succeeded" }),
      step({ step_index: 2, status: "failed" }),
      step({ step_index: 3, status: "skipped" }),
    ];
    // A skipped step AFTER the failure is "Not reached" — the run stopped.
    expect(stepOutcome(steps[2]!, { status: "failed" }, steps).label).toBe("Not reached");
    // A skipped step on a run that did not fail is a branch not taken.
    const clean = [steps[0]!, step({ step_index: 2, status: "skipped" })];
    expect(stepOutcome(clean[1]!, { status: "succeeded" }, clean).label).toBe("Skipped");
  });

  it("maps the plain statuses to the pack's words", () => {
    const steps = [step({ step_index: 1, status: "succeeded" })];
    expect(stepOutcome(steps[0]!, { status: "succeeded" }, steps)).toMatchObject({
      label: "Done",
      tone: "success",
    });
  });

  it("finds the first failure", () => {
    expect(
      firstFailedIndex([step({ step_index: 1, status: "succeeded" }), step({ step_index: 2, status: "failed" })]),
    ).toBe(2);
    expect(firstFailedIndex([step({ step_index: 1, status: "succeeded" })])).toBeNull();
  });
});

describe("the trace's summary line", () => {
  it("says how many steps finished before the break", () => {
    const steps = [
      step({ step_index: 1, status: "succeeded" }),
      step({ step_index: 2, status: "succeeded" }),
      step({ step_index: 3, status: "failed" }),
    ];
    expect(finishedPrefix(steps, { status: "failed" })).toBe("Steps 1 to 2 finished");
  });

  it("reports a clean run's whole tail, and nothing for an empty trace", () => {
    const steps = [step({ step_index: 1, status: "succeeded" }), step({ step_index: 2, status: "succeeded" })];
    expect(finishedPrefix(steps, { status: "succeeded" })).toBe("All 2 steps finished");
    expect(finishedPrefix([], { status: "succeeded" })).toBeNull();
  });

  it("says nothing when the first step is the one that failed", () => {
    expect(finishedPrefix([step({ step_index: 1, status: "failed" })], { status: "failed" })).toBeNull();
  });
});

describe("time and value formatting", () => {
  it("renders a duration from two stamps", () => {
    expect(durationText("2026-10-16T10:00:00Z", "2026-10-16T10:00:00.500Z")).toBe("500ms");
    expect(durationText("2026-10-16T10:00:00Z", "2026-10-16T10:01:12Z")).toBe("1m 12s");
  });

  it("says a run is still going rather than inventing an end", () => {
    expect(durationText("2026-10-16T10:00:00Z", null)).toBe("still going");
  });

  it("spans sub-second and long durations", () => {
    expect(spanText(300)).toBe("300ms");
    expect(spanText(3_780_000)).toBe("1h 03m");
  });

  it("reads relative time from a fixed clock", () => {
    const now = Date.parse("2026-10-16T12:00:00Z");
    expect(relativeTime("2026-10-16T11:59:50Z", now)).toBe("just now");
    expect(relativeTime("2026-10-16T11:55:00Z", now)).toBe("5 minutes ago");
    expect(relativeTime("2026-10-16T09:00:00Z", now)).toBe("3 hours ago");
  });

  it("shortens an id without losing it", () => {
    expect(shortId("0123456789abcdef")).toBe("01234567");
    expect(shortId("abc")).toBe("abc");
  });

  it("reads a structured output as one honest line", () => {
    expect(compactValue({ ticket_id: "abc", created: true })).toBe("ticket_id abc · created true");
    expect(compactValue(["a", "b"])).toBe("a, b");
    expect(compactValue(null)).toBe("—");
    expect(compactValue({})).toBe("—");
  });

  it("cuts a payload too long for a cell instead of hiding the cut", () => {
    const long = compactValue({ note: "x".repeat(400) });
    expect(long.length).toBeLessThanOrEqual(140);
    expect(long.endsWith("…")).toBe(true);
  });

  it("names a failed step's typed code", () => {
    const failed = step({
      step_index: 1,
      status: "failed",
      error_code: "channel_not_found",
      error: "the channel was renamed",
    });
    expect(stepSummary(failed, stepOutcome(failed, { status: "failed" }, [failed]))).toBe(
      "channel_not_found: the channel was renamed",
    );
  });

  it("pretty-prints the detail blocks", () => {
    expect(prettyJson({ a: 1 })).toBe('{\n  "a": 1\n}');
    expect(prettyJson(undefined)).toBe("—");
  });
});
