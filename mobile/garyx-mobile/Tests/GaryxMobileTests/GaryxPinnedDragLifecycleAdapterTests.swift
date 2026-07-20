import Combine
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxPinnedDragLifecycleAdapterTests: XCTestCase {
    func testRepresentableHasNoSwiftUIObservationBackedge() {
        let controller = GaryxPinnedDragLifecycleController()
        let adapter = GaryxPinnedDragLifecycleAdapter(controller: controller)

        XCTAssertFalse(
            conformsToObservableObject(controller),
            "The imperative drag controller must not publish into SwiftUI while UIViewRepresentable is updating."
        )
        XCTAssertFalse(
            storedPropertyTypeNames(in: adapter).contains { $0.contains("ObservedObject") },
            "The representable must borrow its controller without installing a second SwiftUI subscription."
        )
    }

    private func conformsToObservableObject(_ value: Any) -> Bool {
        value is any ObservableObject
    }

    private func storedPropertyTypeNames(in value: Any) -> [String] {
        Mirror(reflecting: value).children.map { String(reflecting: type(of: $0.value)) }
    }
}
