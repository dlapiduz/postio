import Testing

@testable import PostioAppKit

@Suite struct ComposeFieldOrderTests {
    @Test func shiftTabFromTheBodyGoesToTheSubject() {
        #expect(ComposeField.body.previous(showingCopyFields: false) == .subject)
    }

    @Test func shiftTabFromTheSubjectSkipsHiddenCopyFields() {
        #expect(ComposeField.subject.previous(showingCopyFields: false) == .to)
    }

    @Test func shiftTabFromTheSubjectEntersTheCopyFieldsWhenShown() {
        #expect(ComposeField.subject.previous(showingCopyFields: true) == .bcc)
        #expect(ComposeField.bcc.previous(showingCopyFields: true) == .cc)
        #expect(ComposeField.cc.previous(showingCopyFields: true) == .to)
    }

    @Test func shiftTabAtTheTopStaysPut() {
        #expect(ComposeField.to.previous(showingCopyFields: true) == nil)
    }
}
