import XCTest

@testable import GaryxMobileCore

final class GaryxAgentEnvDraftTests: XCTestCase {
    func testSeededDraftIsUnchangedAndOmitsProviderEnv() {
        // Restored-cache scenario: even seeded from a stripped (empty) env, an
        // untouched draft must resolve to .unchanged so the save omits
        // provider_env and the gateway preserves the stored value.
        let stripped = GaryxAgentEnvDraft.seeded(from: [:])
        XCTAssertEqual(stripped.resolvedIntent(), .unchanged)

        let seeded = GaryxAgentEnvDraft.seeded(from: ["EXISTING": "keep"])
        XCTAssertFalse(seeded.isDirty)
        XCTAssertEqual(seeded.resolvedIntent(), .unchanged)
    }

    func testSeededRowsAreSortedByKey() {
        let draft = GaryxAgentEnvDraft.seeded(from: ["B": "2", "A": "1"])
        XCTAssertEqual(draft.rows.map(\.key), ["A", "B"])
    }

    func testEditingProducesReplaceWithFullMap() {
        var draft = GaryxAgentEnvDraft.seeded(from: ["EXISTING": "keep"])
        draft.addRow()
        let newId = draft.rows.last!.id
        draft.updateKey(id: newId, "NEW")
        draft.updateValue(id: newId, "value")
        XCTAssertEqual(
            draft.resolvedIntent(),
            .replace(["EXISTING": "keep", "NEW": "value"])
        )
    }

    func testRemovingAllRowsClears() {
        var draft = GaryxAgentEnvDraft.seeded(from: ["A": "1"])
        draft.removeRow(id: draft.rows[0].id)
        XCTAssertEqual(draft.resolvedIntent(), .clear)
    }

    func testCurrentEnvMapDropsEmptyKeysAndLastRowWins() {
        var draft = GaryxAgentEnvDraft.empty
        draft.addRow()
        let a = draft.rows.last!.id
        draft.updateKey(id: a, "DUP")
        draft.updateValue(id: a, "first")
        draft.addRow()
        let b = draft.rows.last!.id
        draft.updateKey(id: b, "DUP")
        draft.updateValue(id: b, "second")
        draft.addRow()
        let c = draft.rows.last!.id
        draft.updateKey(id: c, "   ")
        draft.updateValue(id: c, "ignored")
        XCTAssertEqual(draft.currentEnvMap(), ["DUP": "second"])
    }

    func testEmptyValueIsPreserved() {
        var draft = GaryxAgentEnvDraft.empty
        draft.addRow()
        let id = draft.rows.last!.id
        draft.updateKey(id: id, "BLANK")
        draft.updateValue(id: id, "")
        XCTAssertEqual(draft.resolvedIntent(), .replace(["BLANK": ""]))
    }

    func testReseedIfPristineReplacesWhenNotDirty() {
        // A late authoritative fetch re-seeds a pristine draft (fixes the
        // restored-cache stripped seed) but must not clobber user edits.
        var pristine = GaryxAgentEnvDraft.seeded(from: [:])
        pristine.reseedIfPristine(from: ["AUTH": "value"])
        XCTAssertEqual(pristine.rows.map(\.key), ["AUTH"])
        XCTAssertFalse(pristine.isDirty)
        XCTAssertEqual(pristine.resolvedIntent(), .unchanged)

        var edited = GaryxAgentEnvDraft.seeded(from: [:])
        edited.addRow()
        edited.updateKey(id: edited.rows[0].id, "USER")
        edited.updateValue(id: edited.rows[0].id, "typed")
        edited.reseedIfPristine(from: ["AUTH": "value"])
        XCTAssertEqual(edited.rows.map(\.key), ["USER"])
    }

    func testIsValidKey() {
        XCTAssertTrue(GaryxAgentEnvDraft.isValidKey("OPENAI_API_KEY"))
        XCTAssertTrue(GaryxAgentEnvDraft.isValidKey("_X1"))
        XCTAssertFalse(GaryxAgentEnvDraft.isValidKey(""))
        XCTAssertFalse(GaryxAgentEnvDraft.isValidKey("1BAD"))
        XCTAssertFalse(GaryxAgentEnvDraft.isValidKey("HAS SPACE"))
        XCTAssertFalse(GaryxAgentEnvDraft.isValidKey("HAS=EQ"))
    }
}
