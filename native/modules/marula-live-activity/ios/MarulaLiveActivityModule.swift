// ActivityKit bridge — the iOS half of the live timer.
//
// Purpose: mirror the ONE server-backed timer entity (GET /timers/active)
// onto the Dynamic Island and the lock screen. The JS side derives the typed
// snapshot (src/live/timer-snapshot.ts) and calls start/update/end; this
// module owns the ActivityKit lifecycle and reports the ActivityKit push
// token through the `onPushToken` event so the app can hand it to a future
// server registration route (the route does not exist yet — recorded gap).
//
// Non-purpose: no timer logic, no state of its own. The session id keys the
// activity; the snapshot is the state.

import ActivityKit
import ExpoModulesCore
import Foundation

public class MarulaLiveActivityModule: Module {
    private var activity: Activity<TimerActivityAttributes>?
    private var tokenTask: Task<Void, Never>?

    public func definition() -> ModuleDefinition {
        Name("MarulaLiveActivity")

        Events("onPushToken")

        Function("isSupported") { () -> Bool in
            if #available(iOS 16.2, *) {
                return ActivityAuthorizationInfo().areActivitiesEnabled
            }
            return false
        }

        AsyncFunction("start") { (payload: [String: Any]) in
            try await self.start(payload)
        }

        AsyncFunction("update") { (payload: [String: Any]) in
            try await self.update(payload)
        }

        AsyncFunction("end") { () in
            await self.end()
        }

        Function("pushToken") { () -> String? in
            guard let data = self.activity?.pushToken else { return nil }
            return data.map { String(format: "%02x", $0) }.joined()
        }

        OnDestroy {
            self.tokenTask?.cancel()
            self.tokenTask = nil
        }
    }

    private func start(_ payload: [String: Any]) async throws {
        if #available(iOS 16.2, *) {
            guard let sessionId = payload["sessionId"] as? String else {
                throw Exception(
                    name: "ERR_LIVE_TIMER_PAYLOAD",
                    description: "The live timer payload is missing sessionId."
                )
            }
            // Reuse the running activity for this session instead of stacking
            // duplicates after a relaunch.
            if let existing = Activity<TimerActivityAttributes>.activities
                .first(where: { $0.attributes.sessionId == sessionId }) {
                activity = existing
                await existing.update(ActivityContent(state: try content(from: payload), staleDate: nil))
                observeToken(existing)
                return
            }
            let attributes = TimerActivityAttributes(sessionId: sessionId)
            let requested = try Activity.request(
                attributes: attributes,
                content: ActivityContent(state: try content(from: payload), staleDate: nil),
                pushType: .token
            )
            activity = requested
            observeToken(requested)
        }
    }

    private func update(_ payload: [String: Any]) async throws {
        if #available(iOS 16.2, *) {
            guard let current = activity else { return }
            await current.update(ActivityContent(state: try content(from: payload), staleDate: nil))
        }
    }

    private func end() async {
        if #available(iOS 16.2, *) {
            guard let current = activity else { return }
            await current.end(nil, dismissalPolicy: .after(Date().addingTimeInterval(30 * 60)))
            activity = nil
            tokenTask?.cancel()
            tokenTask = nil
        }
    }

    private func content(from payload: [String: Any]) throws -> TimerActivityAttributes.ContentState {
        guard
            let state = payload["state"] as? String,
            let segmentStartedAt = date(payload["segmentStartedAt"]),
            let updatedAt = date(payload["updatedAt"])
        else {
            throw Exception(
                name: "ERR_LIVE_TIMER_PAYLOAD",
                description: "The live timer payload has an unparseable state or date."
            )
        }
        return TimerActivityAttributes.ContentState(
            state: state,
            ticketNumber: payload["ticketNumber"] as? String ?? "",
            title: payload["title"] as? String ?? "",
            segmentStartedAt: segmentStartedAt,
            accumulatedSeconds: payload["accumulatedSeconds"] as? Double ?? 0,
            totalSeconds: payload["totalSeconds"] as? Double ?? 0,
            updatedAt: updatedAt
        )
    }

    /// JS `toISOString()` carries milliseconds; the default formatter rejects
    /// them, so accept both shapes.
    private static let iso8601Fractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let iso8601: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    private func date(_ value: Any?) -> Date? {
        guard let raw = value as? String else { return nil }
        return Self.iso8601Fractional.date(from: raw) ?? Self.iso8601.date(from: raw)
    }

    private func observeToken(_ activity: Activity<TimerActivityAttributes>) {
        tokenTask?.cancel()
        let sessionId = activity.attributes.sessionId
        tokenTask = Task { [weak self] in
            for await tokenData in activity.pushTokenUpdates {
                let hex = tokenData.map { String(format: "%02x", $0) }.joined()
                self?.sendEvent("onPushToken", ["sessionId": sessionId, "token": hex])
            }
        }
    }
}
