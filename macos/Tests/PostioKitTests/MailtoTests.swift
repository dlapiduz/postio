import Foundation
import Testing
@testable import PostioKit

@Suite("mailto:")
struct MailtoTests {
    @Test("a plain address is the recipient")
    func plainAddress() {
        let parsed = Mailto(URL(string: "mailto:ada@example.com")!)
        #expect(parsed?.to == ["ada@example.com"])
        #expect(parsed?.subject == nil)
    }

    @Test("several recipients are comma-separated, as RFC 6068 says")
    func severalRecipients() {
        let parsed = Mailto(URL(string: "mailto:ada@example.com,grace@example.net")!)
        #expect(parsed?.to == ["ada@example.com", "grace@example.net"])
    }

    @Test("a subject comes from the query, percent-decoded")
    func subject() {
        let parsed = Mailto(URL(string: "mailto:ada@example.com?subject=Cost%20report")!)
        #expect(parsed?.subject == "Cost report")
    }

    @Test("a body comes across too")
    func body() {
        let parsed = Mailto(URL(string: "mailto:ada@example.com?body=Hello%20there")!)
        #expect(parsed?.body == "Hello there")
    }

    @Test("cc and bcc are recipients of their own kind")
    func ccAndBcc() {
        let url = URL(string: "mailto:ada@example.com?cc=grace@example.net&bcc=alan@example.org")!
        let parsed = Mailto(url)
        #expect(parsed?.cc == ["grace@example.net"])
        #expect(parsed?.bcc == ["alan@example.org"])
    }

    @Test("an address in the query joins the ones in the path")
    func toInQuery() {
        let url = URL(string: "mailto:ada@example.com?to=grace@example.net")!
        #expect(Mailto(url)?.to == ["ada@example.com", "grace@example.net"])
    }

    @Test("a bare mailto: with no address is still a request to compose")
    func noAddress() {
        // A bare `mailto:` is what "New Message" elsewhere sends, not a malformed URL.
        let parsed = Mailto(URL(string: "mailto:")!)
        #expect(parsed != nil)
        #expect(parsed?.to.isEmpty == true)
    }

    @Test("a URL of another scheme is not a mailto")
    func wrongScheme() {
        #expect(Mailto(URL(string: "https://example.com")!) == nil)
    }

    @Test("no header this build does not know is invented")
    func unknownHeadersIgnored() {
        // Postio reads the four headers a composer has fields for and drops the rest rather than
        // honouring them blindly (RFC 6068).
        let url = URL(string: "mailto:ada@example.com?from=someone@example.net&subject=Hi")!
        let parsed = Mailto(url)
        #expect(parsed?.subject == "Hi")
        #expect(parsed?.to == ["ada@example.com"], "`from` did not become a recipient")
    }
}
