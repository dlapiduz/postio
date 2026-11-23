import AppKit
import Testing
@testable import PostioAppKit
@testable import PostioKit

@MainActor
@Suite struct ViewTreeFocusTests {
    @Test func findsTheFieldInsideItsOwnToolbarItemFirst() {
        let window = NSView()
        let item = NSView()
        let leaf = NSView()
        let mine = NSTextField()
        item.addSubview(leaf)
        item.addSubview(mine)
        let elsewhere = NSView()
        let other = NSTextField()
        elsewhere.addSubview(other)
        window.addSubview(elsewhere)  // added first: found first in a naive walk
        window.addSubview(item)

        #expect(ViewTreeFocus.nearestTextField(from: leaf) === mine)
    }

    @Test func aLabelIsNotSomewhereToTypeInto() {
        // `labelWithString` fields are NSTextFields too; focusing a label would swallow the
        // keyboard.
        let item = NSView()
        let leaf = NSView()
        let label = NSTextField(labelWithString: "Search mail")
        let field = NSTextField()
        item.addSubview(leaf)
        item.addSubview(label)
        item.addSubview(field)

        #expect(ViewTreeFocus.nearestTextField(from: leaf) === field)
    }

    @Test func theClimbIsBounded() {
        // Past the toolbar item "nearest" means nothing: a chain longer than the limit answers
        // nothing.
        var top = NSView()
        let field = NSTextField()
        top.addSubview(field)
        var leaf = top
        for _ in 0..<12 {
            let next = NSView()
            leaf.addSubview(next)
            leaf = next
        }
        _ = top
        #expect(ViewTreeFocus.nearestTextField(from: leaf, climbing: 8) == nil)
    }

    @Test func noFieldAnywhereIsNil() {
        let leaf = NSView()
        let parent = NSView()
        parent.addSubview(leaf)
        #expect(ViewTreeFocus.nearestTextField(from: leaf) == nil)
    }
}
