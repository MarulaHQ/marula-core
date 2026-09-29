import ActivityKit
import SwiftUI
import WidgetKit

/// The brand accent, mirroring native/src/ui/tokens.ts (`primary` #E05D38).
private let marulaPrimary = Color(red: 224 / 255, green: 93 / 255, blue: 56 / 255)

/// Brand-adjacent ink for the in-island surfaces (readable on the system's
/// black island in both appearances).
private let marulaInk = Color(red: 51 / 255, green: 51 / 255, blue: 51 / 255)
private let marulaPaper = Color(red: 232 / 255, green: 235 / 255, blue: 237 / 255)
private let marulaDark = Color(red: 28 / 255, green: 36 / 255, blue: 51 / 255)

/// The one Live Activity for a running job timer.
///
/// Presentation rules follow Apple's Live Activities HIG and the WWDC
/// "Design dynamic Live Activities" session:
/// - Compact reads as ONE piece: an identity mark leading, the live clock
///   trailing, no padding against the sensor, heavier type.
/// - Minimal carries information, never just a logo: the live clock.
/// - Expanded is the context moment (the system shows it briefly on start,
///   and on touch-and-hold): what is timed, for which job, since when.
/// - The Lock Screen card repeats the same information with a 14pt margin.
struct LiveTimerWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: TimerActivityAttributes.self) { context in
            LockScreenTimer(context: context)
                .activityBackgroundTint(marulaDark)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    HStack(spacing: 6) {
                        Image(systemName: iconName(context.state))
                            .font(.title2)
                            .foregroundStyle(marulaPrimary)
                        Text(shortTicket(context.state.ticketNumber))
                            .font(.caption.weight(.heavy))
                            .foregroundStyle(marulaPrimary)
                    }
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ElapsedClock(state: context.state)
                        .font(.title2.weight(.semibold).monospacedDigit())
                        .foregroundStyle(.white)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(jobTitle(context.state))
                            .font(.headline)
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        Text(caption(context.state))
                            .font(.caption)
                            .foregroundStyle(.white.opacity(0.75))
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } compactLeading: {
                // Identity mark inside a small brand capsule (the Wareg/Uber
                // compact pattern): the filled mark reads as "this app" and
                // the trailing clock completes one piece of information.
                Image(systemName: iconName(context.state))
                    .font(.caption.weight(.bold))
                    .foregroundStyle(.white)
                    .padding(5)
                    .background(marulaPrimary, in: Capsule())
            } compactTrailing: {
                CompactClockBody(state: context.state)
                    .font(.body.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.white)
                    .lineLimit(1)
            } minimal: {
                // Information, not a logo (Apple's Timer example): the live
                // clock keeps working even in the smallest form.
                CompactClockBody(state: context.state)
                    .font(.caption2.weight(.semibold).monospacedDigit())
                    .foregroundStyle(marulaPrimary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
            .widgetURL(URL(string: "marula://timer/\(context.attributes.sessionId)"))
            .keylineTint(marulaPrimary)
        }
    }
}

/// Lock Screen card: ticket, job, state, and the live clock. Keeps the same
/// relative order as the expanded presentation so the two read as one design.
private struct LockScreenTimer: View {
    let context: ActivityViewContext<TimerActivityAttributes>

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: iconName(context.state))
                .font(.title2)
                .foregroundStyle(marulaPrimary)
            VStack(alignment: .leading, spacing: 3) {
                Text(shortTicket(context.state.ticketNumber))
                    .font(.caption.weight(.heavy))
                    .foregroundStyle(marulaPrimary)
                Text(jobTitle(context.state))
                    .font(.headline)
                    .foregroundStyle(.white)
                    .lineLimit(2)
                Text(shortCaption(context.state))
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.75))
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                ElapsedClock(state: context.state)
                    .font(.title2.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.white)
                Text(context.state.state == "running" ? "running" : "paused")
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.white.opacity(0.7))
            }
        }
        .padding(14)
    }
}

/// The live segment clock: a ticking range the system advances each second.
/// The system picks the format ("0:42", "12:34", "1:02:03"), which reads as a
/// stopwatch next to the identity mark.
private struct SegmentClock: View {
    let state: TimerActivityAttributes.ContentState

    var body: some View {
        Text(timerInterval: state.segmentStartedAt ... Date.distantFuture, countsDown: false)
    }
}

/// Elapsed clock with the whole hours split out, so the ticking text stays
/// MM:SS: "1h12m" (static, updated with the session) + "34:07" (ticking).
/// In Always-On the seconds read "--" until the user looks at the device —
/// that is system behaviour for every ticking surface, not a defect.
private struct ElapsedClock: View {
    let state: TimerActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 4) {
            if let banked = banked(state) {
                Text(banked)
                    .foregroundStyle(.white.opacity(0.6))
            }
            if state.state == "running" {
                Text(timerInterval: state.segmentStartedAt ... Date.distantFuture, countsDown: false)
            } else {
                Text(staticClock(state.totalSeconds))
            }
        }
    }

    private func staticClock(_ seconds: Double) -> String {
        let total = max(0, Int(seconds.rounded()))
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d", hours, minutes, secs)
        }
        return String(format: "%d:%02d", minutes, secs)
    }
}

/// Compact clock: a ticking stopwatch while running, a frozen total while
/// paused (a paused timer must not look like it is counting). The limited
/// width signal for sideways use belongs here once the SDK exposes it.
private struct CompactClockBody: View {
    let state: TimerActivityAttributes.ContentState

    var body: some View {
        if state.state == "running" {
            Text(timerInterval: state.segmentStartedAt ... Date.distantFuture, countsDown: false)
        } else {
            Text(clock(state.totalSeconds))
        }
    }

    private func clock(_ seconds: Double) -> String {
        let total = max(0, Int(seconds.rounded()))
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d", hours, minutes, secs)
        }
        return String(format: "%d:%02d", minutes, secs)
    }
}

/// Banked time before the current run, shown only once it is worth a label.
private func banked(_ state: TimerActivityAttributes.ContentState) -> String? {
    let seconds = Int(state.accumulatedSeconds.rounded())
    guard seconds >= 60 else { return nil }
    let hours = seconds / 3600
    let minutes = (seconds % 3600) / 60
    if hours > 0 {
        return String(format: "%d:%02d", hours, minutes)
    }
    return "\(minutes)m"
}

/// The job line: the ticket title, or a plain fallback.
private func jobTitle(_ state: TimerActivityAttributes.ContentState) -> String {
    let title = state.title.trimmingCharacters(in: .whitespaces)
    return title.isEmpty ? "Job timer" : title
}

/// "Job timer running since 14:05" / "Job timer paused" — the sentence that
/// answers "what is this timer?" in the expanded and Lock Screen layouts.
private func caption(_ state: TimerActivityAttributes.ContentState) -> String {
    if state.state == "running" {
        return "Job timer running since \(sessionClock(state))"
    }
    return "Job timer paused"
}

/// Lock Screen caption, short enough for one line at the 14pt margin.
private func shortCaption(_ state: TimerActivityAttributes.ContentState) -> String {
    if state.state == "running" {
        return "Running since \(sessionClock(state))"
    }
    return "Paused"
}

/// Short ticket hint: the last four characters of the number
/// (e.g. DEMO1034 -> 1034).
private func shortTicket(_ ticketNumber: String) -> String {
    let trimmed = ticketNumber.trimmingCharacters(in: .whitespaces)
    if trimmed.isEmpty { return "TIMER" }
    return String(trimmed.suffix(4))
}

private func sessionClock(_ state: TimerActivityAttributes.ContentState) -> String {
    let formatter = DateFormatter()
    formatter.dateFormat = "HH:mm"
    return formatter.string(from: state.segmentStartedAt)
}

private func iconName(_ state: TimerActivityAttributes.ContentState) -> String {
    state.state == "running" ? "timer" : "pause.fill"
}
