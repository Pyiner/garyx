import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxRecentThreadFilterStorageTests: XCTestCase {
    private let productionKey = "garyx.mobile.recentThreadFilter"

    func testPersistenceValuesAreStableInternalLiterals() {
        XCTAssertEqual(GaryxRecentThreadFilterStorage.persistenceValue(for: .all), "all")
        XCTAssertEqual(GaryxRecentThreadFilterStorage.persistenceValue(for: .nonTask), "nonTask")
        XCTAssertNotEqual(
            GaryxRecentThreadFilterStorage.persistenceValue(for: .nonTask),
            GaryxRecentThreadFilter.nonTask.displayName
        )
        XCTAssertNotEqual(
            GaryxRecentThreadFilterStorage.persistenceValue(for: .nonTask),
            GaryxRecentThreadFilter.nonTask.tasksQueryValue
        )
    }

    func testSaveAndLoadRoundTripBothFiltersAcrossStoreCalls() throws {
        let fixture = try makeDefaults()
        defer { fixture.defaults.removePersistentDomain(forName: fixture.suiteName) }

        for filter in GaryxRecentThreadFilter.homeMenuOptions {
            GaryxRecentThreadFilterStorage.save(
                filter,
                defaults: fixture.defaults,
                key: productionKey
            )
            XCTAssertEqual(
                GaryxRecentThreadFilterStorage.load(
                    defaults: fixture.defaults,
                    key: productionKey
                ),
                filter
            )
        }
    }

    func testMissingEmptyAndUnknownValuesFallBackToAllWithoutWriting() throws {
        let fixture = try makeDefaults()
        defer { fixture.defaults.removePersistentDomain(forName: fixture.suiteName) }

        XCTAssertNil(fixture.defaults.object(forKey: productionKey))
        XCTAssertEqual(
            GaryxRecentThreadFilterStorage.load(
                defaults: fixture.defaults,
                key: productionKey
            ),
            .all
        )
        XCTAssertNil(fixture.defaults.object(forKey: productionKey))

        for value in ["", "tasksOnly"] {
            fixture.defaults.set(value, forKey: productionKey)
            XCTAssertEqual(
                GaryxRecentThreadFilterStorage.load(
                    defaults: fixture.defaults,
                    key: productionKey
                ),
                .all
            )
            XCTAssertEqual(fixture.defaults.string(forKey: productionKey), value)
        }
    }

    func testInjectedKeysDoNotPolluteEachOther() throws {
        let fixture = try makeDefaults()
        defer { fixture.defaults.removePersistentDomain(forName: fixture.suiteName) }

        GaryxRecentThreadFilterStorage.save(
            .nonTask,
            defaults: fixture.defaults,
            key: productionKey
        )

        XCTAssertEqual(
            GaryxRecentThreadFilterStorage.load(
                defaults: fixture.defaults,
                key: productionKey
            ),
            .nonTask
        )
        XCTAssertEqual(
            GaryxRecentThreadFilterStorage.load(
                defaults: fixture.defaults,
                key: "synthetic.test.recentFilter"
            ),
            .all
        )
    }

    private func makeDefaults() throws -> (suiteName: String, defaults: UserDefaults) {
        let suiteName = "GaryxRecentThreadFilterStorageTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        return (suiteName, defaults)
    }
}
