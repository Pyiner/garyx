import Foundation
import XCTest

final class GaryxThreadListCleanupContractTests: XCTestCase {
    func testRemovedThreadListLayersDoNotReturnToProductSources() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let roots = [
            packageRoot.appendingPathComponent("App/GaryxMobile", isDirectory: true),
            packageRoot.appendingPathComponent("Sources/GaryxMobileCore", isDirectory: true),
        ]
        let swiftFiles = roots.flatMap { root -> [URL] in
            guard let enumerator = FileManager.default.enumerator(
                at: root,
                includingPropertiesForKeys: nil
            ) else {
                return []
            }
            return enumerator.compactMap { entry in
                guard let url = entry as? URL, url.pathExtension == "swift" else { return nil }
                return url
            }
        }
        XCTAssertFalse(swiftFiles.isEmpty)
        let productSource = try swiftFiles
            .map { try String(contentsOf: $0, encoding: .utf8) }
            .joined(separator: "\n")

        for removedSymbol in [
            "GaryxHomeThreadButton",
            "GaryxSidebarThreadButton",
            "GaryxThreadsPage",
            "func listThreads(",
        ] {
            XCTAssertFalse(
                productSource.contains(removedSymbol),
                "removed thread-list layer returned: \(removedSymbol)"
            )
        }
    }
}
