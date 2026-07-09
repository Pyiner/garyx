import XCTest
@testable import GaryxMobileCore

/// #TASK-2038 P2: unknown (forward-compat) render rows must round-trip
/// decode → encode without losing the original wire payload. Desktop keeps
/// the raw row object (stream.ts only validates the snapshot's top level),
/// and mobile persists `renderSnapshot` through `GaryxTranscriptCache`, so
/// a lossy fallback would make forward-compat loss *persistent*: a newer
/// server's row would be cached back as a `{kind:unknown,id}` husk.
final class GaryxRenderRowForwardCompatTests: XCTestCase {
    /// codex's review probe: a future row kind carrying a payload this
    /// build knows nothing about.
    private let futureWidgetJSON = """
    {"kind":"future_widget","id":"row-9","label":"widget","payload":{"answer":42,"nested":{"deep":true},"tags":["a","b"]}}
    """

    func testUnknownRowRoundTripPreservesTheOriginalWirePayload() throws {
        let original = Data(futureWidgetJSON.utf8)
        let row = try JSONDecoder().decode(GaryxRenderRow.self, from: original)

        let reencoded = try JSONEncoder().encode(row)
        XCTAssertEqual(
            try JSONDecoder().decode(GaryxJSONValue.self, from: reencoded),
            try JSONDecoder().decode(GaryxJSONValue.self, from: original),
            "unknown row kinds must re-encode their original wire object verbatim (desktop parity)"
        )
    }

    func testSnapshotDecodeKeepsUnknownRowPayloadsForTheCacheCodec() throws {
        // The snapshot decode path (lossy rows array) is what both the live
        // stream and the persisted cache go through; the unknown row must
        // survive it payload-intact, alongside known kinds.
        let snapshotJSON = """
        {"based_on_seq":7,"rows":[\
        {"kind":"user_turn","id":"turn-1","user":{"id":"history:0","seq":1,"role":"user"},"activity":[]},\
        \(futureWidgetJSON)\
        ],"tailActivity":"none","progress_locus":"none","rows_hash":"1001"}
        """
        let snapshot = try JSONDecoder().decode(GaryxRenderSnapshot.self, from: Data(snapshotJSON.utf8))
        XCTAssertEqual(snapshot.rows.count, 2, "the unknown row must not be dropped")

        let reencoded = try JSONEncoder().encode(snapshot.rows[1])
        XCTAssertEqual(
            try JSONDecoder().decode(GaryxJSONValue.self, from: reencoded),
            try JSONDecoder().decode(GaryxJSONValue.self, from: Data(futureWidgetJSON.utf8)),
            "a cached snapshot must write the unknown row back with its full original body"
        )
    }

    func testLegacyLossyUnknownRowStillDecodesAndKeepsItsId() throws {
        // Caches written by builds before the fix hold the lossy husk shape;
        // it must keep decoding, and its id must keep anchoring delta
        // reassembly carry-forward.
        let legacyJSON = Data(#"{"kind":"unknown","id":"row-x"}"#.utf8)
        let row = try JSONDecoder().decode(GaryxRenderRow.self, from: legacyJSON)
        let reencoded = try JSONEncoder().encode(row)
        XCTAssertEqual(
            try JSONDecoder().decode(GaryxJSONValue.self, from: reencoded),
            try JSONDecoder().decode(GaryxJSONValue.self, from: legacyJSON)
        )
    }
}
