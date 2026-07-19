import Combine
import Foundation
import os
import UIKit

#if DEBUG
@MainActor
final class GaryxHomeScrollPerformanceProbe: NSObject, ObservableObject {
    static let shared = GaryxHomeScrollPerformanceProbe()

    private let log = OSLog(subsystem: "com.garyx.mobile", category: "HomeScroll")
    private let logger = Logger(subsystem: "com.garyx.mobile", category: "HomeScroll")
    private var displayLink: CADisplayLink?
    private var objectWillChangeCancellable: AnyCancellable?
    private var windowStartTimestamp: CFTimeInterval?
    private var previousDisplayTimestamp: CFTimeInterval?
    private var elapsedFrameTime: CFTimeInterval = 0
    private var hitchTime: CFTimeInterval = 0
    private var maxFrameInterval: CFTimeInterval = 0
    private var frameBudget: CFTimeInterval = 1.0 / 60.0
    @Published private(set) var isRecording = false
    @Published private(set) var latestReport: GaryxHomeScrollPerformanceReport?
    private(set) var rootBodyCount = 0
    private(set) var homeBodyCount = 0
    private(set) var rowBodyCount = 0
    private(set) var modelPublishCount = 0
    private(set) var homeListStoreApplyCount = 0

    func attachModelObjectWillChange(_ publisher: ObservableObjectPublisher) {
        guard objectWillChangeCancellable == nil else { return }
        objectWillChangeCancellable = publisher.sink { [weak self] _ in
            Task { @MainActor in
                self?.markModelObjectWillChange()
            }
        }
    }

    func beginWindow(label: StaticString = "home_scroll_probe") {
        resetCounters()
        latestReport = nil
        isRecording = true
        os_signpost(.begin, log: log, name: "GaryxHomeScrollProbe", "%{public}s", "\(label)")
        displayLink?.invalidate()
        let link = CADisplayLink(target: self, selector: #selector(stepDisplayLink(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 60)
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @discardableResult
    func endWindow() -> GaryxHomeScrollPerformanceReport {
        displayLink?.invalidate()
        displayLink = nil
        let report = GaryxHomeScrollPerformanceReport(
            rootBodyCount: rootBodyCount,
            homeBodyCount: homeBodyCount,
            rowBodyCount: rowBodyCount,
            modelPublishCount: modelPublishCount,
            homeListStoreApplyCount: homeListStoreApplyCount,
            hitchTimeRatio: elapsedFrameTime > 0 ? hitchTime / elapsedFrameTime : 0,
            maxFrameInterval: maxFrameInterval,
            worstFrameDelta: max(0, maxFrameInterval - frameBudget)
        )
        isRecording = false
        latestReport = report
        os_signpost(
            .end,
            log: log,
            name: "GaryxHomeScrollProbe",
            "root=%{public}d home=%{public}d row=%{public}d model=%{public}d store_apply=%{public}d hitch_ratio=%{public}.4f max_frame_interval=%{public}.6f worst_frame_delta=%{public}.6f",
            report.rootBodyCount,
            report.homeBodyCount,
            report.rowBodyCount,
            report.modelPublishCount,
            report.homeListStoreApplyCount,
            report.hitchTimeRatio,
            report.maxFrameInterval,
            report.worstFrameDelta
        )
        logger.info(
            "GARYX_HOME_SCROLL_PROBE root_body=\(report.rootBodyCount, privacy: .public) home_body=\(report.homeBodyCount, privacy: .public) row_body=\(report.rowBodyCount, privacy: .public) model_publish=\(report.modelPublishCount, privacy: .public) home_store_apply=\(report.homeListStoreApplyCount, privacy: .public) hitch_time_ratio=\(report.hitchTimeRatio, privacy: .public) max_frame_interval=\(report.maxFrameInterval, privacy: .public) worst_frame_delta=\(report.worstFrameDelta, privacy: .public)"
        )
        let line = report.machineReadableLine
        print(line)
        writeReport(line)
        return report
    }

    func markRootBody() {
        rootBodyCount += 1
        os_signpost(.event, log: log, name: "GaryxRootNavigationView.body")
    }

    func markHomeBody() {
        homeBodyCount += 1
        os_signpost(.event, log: log, name: "GaryxHomeThreadListView.body")
    }

    func markRowBody() {
        rowBodyCount += 1
        os_signpost(.event, log: log, name: "GaryxThreadListRowButton.body")
    }

    func markModelObjectWillChange() {
        modelPublishCount += 1
        os_signpost(.event, log: log, name: "GaryxMobileModel.objectWillChange")
    }

    func markHomeListStoreApply() {
        homeListStoreApplyCount += 1
        os_signpost(.event, log: log, name: "GaryxHomeThreadListStore.apply")
    }

    @objc private func stepDisplayLink(_ link: CADisplayLink) {
        if windowStartTimestamp == nil {
            windowStartTimestamp = link.timestamp
        }
        let scheduledInterval = max(0, link.targetTimestamp - link.timestamp)
        if scheduledInterval > 0 {
            frameBudget = min(max(scheduledInterval, 1.0 / 120.0), 1.0 / 30.0)
        }
        guard let previousDisplayTimestamp else {
            self.previousDisplayTimestamp = link.timestamp
            return
        }
        let interval = max(0, link.timestamp - previousDisplayTimestamp)
        self.previousDisplayTimestamp = link.timestamp
        elapsedFrameTime += interval
        maxFrameInterval = max(maxFrameInterval, interval)
        let hitchThreshold = frameBudget * 1.5
        if interval > hitchThreshold {
            hitchTime += interval - frameBudget
        }
    }

    private func resetCounters() {
        windowStartTimestamp = nil
        previousDisplayTimestamp = nil
        elapsedFrameTime = 0
        hitchTime = 0
        maxFrameInterval = 0
        rootBodyCount = 0
        homeBodyCount = 0
        rowBodyCount = 0
        modelPublishCount = 0
        homeListStoreApplyCount = 0
    }

    private func writeReport(_ line: String) {
        guard let cacheURL = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first else { return }
        let reportURL = cacheURL.appendingPathComponent("garyx-home-scroll-probe.txt", isDirectory: false)
        try? line.appending("\n").write(to: reportURL, atomically: true, encoding: .utf8)
    }
}

struct GaryxHomeScrollPerformanceReport: Equatable {
    var rootBodyCount: Int
    var homeBodyCount: Int
    var rowBodyCount: Int
    var modelPublishCount: Int
    var homeListStoreApplyCount: Int
    var hitchTimeRatio: Double
    var maxFrameInterval: Double
    var worstFrameDelta: Double

    var machineReadableLine: String {
        "GARYX_HOME_SCROLL_PROBE root_body=\(rootBodyCount) home_body=\(homeBodyCount) row_body=\(rowBodyCount) model_publish=\(modelPublishCount) home_store_apply=\(homeListStoreApplyCount) hitch_time_ratio=\(hitchTimeRatio) max_frame_interval=\(maxFrameInterval) worst_frame_delta=\(worstFrameDelta)"
    }
}
#endif
