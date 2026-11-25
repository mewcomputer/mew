import XCTest
import MewMobileCore
@testable import mew

final class ConversationActivityTests: XCTestCase {
    private func tool(_ id: String, _ name: String, input: String = "{}", state: String? = "completed") -> MessagePart {
        MessagePart(id: id, kind: .toolCall, text: nil, toolName: name, toolState: state,
            toolInput: input, toolOutput: "done", toolError: state == "error" ? "failed" : nil,
            toolCallId: id, toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
    }

    private func text(_ id: String, _ value: String, kind: PartKind = .text) -> MessagePart {
        MessagePart(id: id, kind: kind, text: value, toolName: nil, toolState: nil,
            toolInput: nil, toolOutput: nil, toolError: nil, toolCallId: nil,
            toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: nil)
    }

    private func message(_ id: String, _ parts: [MessagePart], role: String = "assistant", steered: Bool = false, cancelled: Bool = false) -> ChatMessage {
        ChatMessage(id: id, role: role, parts: parts, assistantMeta: nil, steered: steered, cancelled: cancelled)
    }

    func testConsecutiveToolsGroupAcrossMessagesAndKeepSurroundingText() {
        let original = [
            message("a", [text("intro", "Checking…"), tool("edit", "write")]),
            message("b", [tool("command-1", "bash")]),
            message("c", [tool("command-2", "bash"), text("answer", "Done.")])
        ]
        let rows = ConversationItem.grouped(messages: original)
        XCTAssertEqual(rows.count, 3)
        XCTAssertEqual(rows[0].message.parts.map(\.id), ["intro"])
        XCTAssertEqual(rows[1].message.parts.map(\.id), ["edit", "command-1", "command-2"])
        XCTAssertEqual(rows[1].sourceMessageIds, ["a", "b", "c"])
        XCTAssertEqual(rows[2].message.parts.map(\.id), ["answer"])
        XCTAssertEqual(original.map { $0.parts.count }, [2, 1, 2])
    }

    func testTextReasoningErrorsAndUserMessagesSeparateGroups() {
        for boundary in [text("boundary", "Explanation"), text("boundary", "Thinking", kind: .reasoning), text("boundary", "Failure", kind: .error)] {
            let rows = ConversationItem.grouped(messages: [message("a", [tool("one", "bash"), boundary, tool("two", "bash")])])
            XCTAssertEqual(rows.map { $0.message.parts.count }, [1, 1, 1])
        }
        for role in ["user", "system"] {
            let rows = ConversationItem.grouped(messages: [message("a", [tool("one", "bash")]), message("boundary", [], role: role), message("b", [tool("two", "bash")])])
            XCTAssertEqual(rows.count, 2)
            XCTAssertEqual(rows.map { $0.message.parts.map(\.id) }, [["one"], ["two"]])
        }
    }

    func testAppendingKeepsTheGroupIdentityAndPrependingRetainsReadingOrigins() {
        let current = [message("a", [tool("one", "bash")]), message("b", [tool("two", "bash")])]
        let rows = ConversationItem.grouped(messages: current)
        let appended = ConversationItem.grouped(messages: current + [message("c", [tool("three", "bash")])])
        XCTAssertEqual(rows[0].id, appended[0].id)
        XCTAssertEqual(appended[0].sourceMessageIds, ["a", "b", "c"])
        let prepended = ConversationItem.grouped(messages: [message("older", [tool("zero", "read")])] + current)
        XCTAssertEqual(prepended.count, 1)
        XCTAssertTrue(prepended[0].sourceMessageIds.contains("a"))
        XCTAssertEqual(prepended[0].message.parts.map(\.id), ["zero", "one", "two"])
    }

    func testEmptyAssistantTextDoesNotSplitAnActivityRun() {
        let rows = ConversationItem.grouped(messages: [message("a", [tool("one", "bash")]), message("empty", [text("empty-text", "")]), message("b", [tool("two", "bash")])])
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].message.parts.map(\.id), ["one", "two"])
        XCTAssertTrue(ConversationItem.grouped(messages: []).isEmpty)
    }

    func testSummaryCountsDistinctEditedPathsAndActualCommandCalls() {
        let parts = [
            tool("edit", "edit_str_replace", input: "{\"path\":\"src/main.rs\"}"),
            tool("write", "write", input: "{\"path\":\"./src/main.rs\"}"),
            tool("one", "bash", input: "{\"command\":\"cargo test\"}"),
            tool("two", "shell", input: "{\"command\":\"cargo fmt\"}")
        ]
        XCTAssertEqual(ToolActivity.summary(parts), "Edited 1 file and ran 2 commands")
        XCTAssertEqual(ToolActivity.summary(Array(parts.suffix(1))), "Ran 1 command")
        XCTAssertEqual(ToolActivity.summary(Array(parts.suffix(2)) + [tool("three", "bash")]), "Ran 3 commands")
    }

    func testHashlineSummaryCountsAllTargetFilesOnce() throws {
        let patch = "[src/main.rs#A1B2]\nDEL 1\n[src/lib.rs#C3D4]\nDEL 2\n[src/main.rs#A1B2]\nINS.TAIL:\n+done"
        let input = try XCTUnwrap(String(data: JSONSerialization.data(withJSONObject: ["patch": patch]), encoding: .utf8))
        XCTAssertEqual(ToolActivity.summary([tool("patch", "edit_hashline", input: input)]), "Edited 2 files")
    }

    func testPendingAndFailedCallsDoNotClaimSuccessfulEditsOrCommands() {
        let parts = [tool("edit", "write", input: "{\"path\":\"main.rs\"}", state: "error"), tool("one", "bash", state: "pending")]
        XCTAssertEqual(ToolActivity.summary(parts), "1 file edit and 1 command")
        XCTAssertEqual(ToolActivity.status(parts), "1 pending, 1 failed")
        XCTAssertEqual(ToolActivity.status([tool("legacy", "bash", state: nil)]), "1 pending")
    }

    func testReadSearchAndUnknownToolsHaveHonestFallbackSummaries() {
        XCTAssertEqual(ToolActivity.summary([tool("read", "read", input: "{\"path\":\"main.rs\"}"), tool("grep", "grep")]), "Read 1 file and ran 1 search")
        XCTAssertEqual(ToolActivity.summary([tool("unknown", "mcp__docs__lookup", input: "not JSON")]), "Made 1 tool call")
        XCTAssertEqual(ToolActivity.summary([tool("edit", "write", input: "not JSON")]), "Made 1 edit")
    }

    func testExpandedRowsShowCommandLinesAndFileNames() {
        XCTAssertEqual(ToolActivity.title(tool("cmd", "bash", input: "{\"command\":\"cargo test -p mew\"}")), "Ran cargo test -p mew")
        XCTAssertEqual(ToolActivity.title(tool("edit", "edit_str_replace", input: "{\"path\":\"src/main.rs\"}")), "Edited main.rs")
        XCTAssertEqual(ToolActivity.title(tool("cmd", "bash", input: "{\"command\":\"cargo test\"}", state: "running")), "Running cargo test")
        XCTAssertEqual(ToolActivity.title(tool("unknown", "mcp__docs__lookup", input: "not JSON")), "mcp__docs__lookup")
    }

    func testCachedDisplayMetadataTracksInputChangesAndLateToolNames() {
        let cache = ToolActivity.Cache()
        let original = tool("same", "write", input: "{\"path\":\"first.rs\"}")
        XCTAssertEqual(ToolActivity.summary([original], cache: cache), "Edited 1 file")
        XCTAssertEqual(ToolActivity.title(original, cache: cache), "Edited first.rs")
        let changed = tool("same", "write", input: "{\"path\":\"second.rs\"}")
        XCTAssertEqual(ToolActivity.title(changed, cache: cache), "Edited second.rs")
        let running = tool("same", "write", input: "{\"path\":\"second.rs\"}", state: "running")
        XCTAssertEqual(ToolActivity.title(running, cache: cache), "Editing second.rs")
        let command = tool("same", "bash", input: "{\"command\":\"cargo test\"}")
        XCTAssertEqual(ToolActivity.title(command, cache: cache), "Ran cargo test")
    }

    func testCompactionPartsAndCancelledEmptyAssistantMessagesSurviveGrouping() {
        let compaction = MessagePart(
            id: "cmp-1", kind: .compaction, text: "Summary", toolName: nil, toolState: nil,
            toolInput: nil, toolOutput: nil, toolError: nil, toolCallId: nil,
            toolTimeStart: nil, toolTimeEnd: nil, toolSensitivity: nil, compactionRemoved: 6
        )
        let items = ConversationItem.grouped(messages: [
            message("u-1", [text("u1", "hi")], role: "user", steered: true),
            message("sys-1", [compaction], role: "system"),
            message("a-1", [], role: "assistant", cancelled: true)
        ])
        XCTAssertEqual(items.count, 3)
        XCTAssertTrue(items[0].message.steered)
        XCTAssertEqual(items[1].message.parts[0].kind, .compaction)
        XCTAssertTrue(items[2].message.cancelled)
    }
}
