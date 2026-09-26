import ActivityKit
import SwiftUI
import WidgetKit

/// The brand accent, mirroring native/src/ui/tokens.ts (`primary` #E05D38).
private let meridianPrimary = Color(red: 224 / 255, green: 93 / 255, blue: 56 / 255)
private let meridianInk = Color(red: 51 / 255, green: 51 / 255, blue: 51 / 255)
private let meridianPaper = Color(red: 232 / 255, green: 235 / 255, blue: 237 / 255)
private let meridianDark = Color(red: 28 / 255, green: 36 / 255, blue: 51 / 255)

struct LiveTimerWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: TimerActivityAttributes.self) { context in
            LockScreenTimer(context: context)
                .activityBackgroundTint(meridianDark)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: iconName(context.state))
                        .foregroundStyle(meridianPrimary)
                        .font(.title2)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ElapsedText(state: context.state)
                        .font(.title2.monospacedDigit())
                        .foregroundStyle(.white)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(context.state.ticketNumber)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(meridianPrimary)
                        Text(context.state.title)
                            .font(.caption)
                            .foregroundStyle(.white.opacity(0.8))
                            .lineLimit(1)
                    }
                }
            } compactLeading: {
                // The compact slot cannot hold the glyph and the "what" at
                // once; the short ticket id answers "what is being timed"
                // (vision review finding 12).
                Text(shortTicket(context.state.ticketNumber))
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(meridianPrimary)
            } compactTrailing: {
                ElapsedText(state: context.state)
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.white)
            } minimal: {
                Image(systemName: iconName(context.state))
                    .foregroundStyle(meridianPrimary)
            }
            .widgetURL(URL(string: "meridian://timer/\(context.attributes.sessionId)"))
            .keylineTint(meridianPrimary)
        }
    }
}

private struct LockScreenTimer: View {
    let context: ActivityViewContext<TimerActivityAttributes>

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: iconName(context.state))
                .font(.title2)
                .foregroundStyle(meridianPrimary)
            VStack(alignment: .leading, spacing: 2) {
                Text(context.state.ticketNumber)
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(meridianPrimary)
                Text(context.state.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.white)
                    .lineLimit(1)
                Text(context.state.state == "running" ? "Timer running" : "Timer paused")
                    .font(.caption2)
                    .foregroundStyle(.white.opacity(0.7))
            }
            Spacer()
            ElapsedText(state: context.state)
                .font(.title3.monospacedDigit())
                .foregroundStyle(.white)
        }
        .padding(14)
    }
}

/// Live-ticking elapsed time: banked seconds + the current segment ticking up
/// while running; a static clock while paused.
private struct ElapsedText: View {
    let state: TimerActivityAttributes.ContentState

    var body: some View {
        if state.state == "running" {
            HStack(spacing: 0) {
                if state.accumulatedSeconds >= 1 {
                    Text(clock(state.accumulatedSeconds))
                }
                Text(timerInterval: state.segmentStartedAt ... Date.distantFuture, countsDown: false)
            }
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
        return String(format: "%02d:%02d", minutes, secs)
    }
}

private func iconName(_ state: TimerActivityAttributes.ContentState) -> String {
    state.state == "running" ? "timer" : "pause.fill"
}

/// Compact-island hint: the last four characters of the ticket number
/// (e.g. DEMO1034 -> 1034).
private func shortTicket(_ ticketNumber: String) -> String {
    let trimmed = ticketNumber.trimmingCharacters(in: .whitespaces)
    if trimmed.isEmpty { return "—" }
    return String(trimmed.suffix(4))
}
