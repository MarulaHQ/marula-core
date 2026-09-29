// Timer Live Activity attributes — the app half of the pair.
//
// Keep in lockstep with native/targets/live-timer/TimerActivityAttributes.swift
// (the widget extension's copy); native/tests/live-activity-parity.test.ts
// fails on drift. Both must declare the same type name, because ActivityKit
// matches the running activity to the widget by that name.

import ActivityKit
import Foundation

public struct TimerActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable {
        /// "running" | "paused"
        public var state: String
        public var ticketNumber: String
        public var title: String
        /// Current segment start; the ticking text anchors here.
        public var segmentStartedAt: Date
        /// Seconds already banked before the current segment.
        public var accumulatedSeconds: Double
        /// Authoritative total seconds at `updatedAt` (used while paused).
        public var totalSeconds: Double
        public var updatedAt: Date

        public init(
            state: String,
            ticketNumber: String,
            title: String,
            segmentStartedAt: Date,
            accumulatedSeconds: Double,
            totalSeconds: Double,
            updatedAt: Date
        ) {
            self.state = state
            self.ticketNumber = ticketNumber
            self.title = title
            self.segmentStartedAt = segmentStartedAt
            self.accumulatedSeconds = accumulatedSeconds
            self.totalSeconds = totalSeconds
            self.updatedAt = updatedAt
        }
    }

    public var sessionId: String

    public init(sessionId: String) {
        self.sessionId = sessionId
    }
}
