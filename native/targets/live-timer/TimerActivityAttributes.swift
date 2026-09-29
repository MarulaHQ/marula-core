// Timer Live Activity attributes — the widget half of the pair.
//
// One truth: the server-backed timer session (GET /timers/active) is mapped
// to this content state by native/src/live/timer-snapshot.ts; the app pushes
// updates and the widget renders them. This file is compiled into the widget
// extension, and native/modules/marula-live-activity/ios/TimerActivityAttributes.swift
// is compiled into the app's module — the two declarations must keep the
// same name and shape or ActivityKit cannot match the activity to the
// widget. native/tests/live-activity-parity.test.ts fails on drift.

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
