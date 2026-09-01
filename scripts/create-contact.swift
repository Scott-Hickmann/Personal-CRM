import Contacts
import Foundation

struct LabeledValue: Decodable {
    let label: String?
    let value: String
}

struct Input: Decodable {
    let containerId: String
    let displayName: String
    let emails: [LabeledValue]
    let phones: [LabeledValue]
    let organization: String
}

let input = try JSONDecoder().decode(Input.self, from: FileHandle.standardInput.readDataToEndOfFile())
let store = CNContactStore()
let semaphore = DispatchSemaphore(value: 0)
var authorized = false
store.requestAccess(for: .contacts) { granted, _ in
    authorized = granted
    semaphore.signal()
}
semaphore.wait()
guard authorized else {
    throw NSError(domain: "PersonalCRM", code: 1, userInfo: [NSLocalizedDescriptionKey: "Contacts access was denied"])
}
guard try store.containers(matching: nil).contains(where: { $0.identifier == input.containerId }) else {
    throw NSError(domain: "PersonalCRM", code: 2, userInfo: [NSLocalizedDescriptionKey: "The configured iCloud container is not writable through Contacts"])
}

let contact = CNMutableContact()
contact.givenName = input.displayName
contact.organizationName = input.organization
contact.emailAddresses = input.emails.map {
    CNLabeledValue(label: $0.label ?? CNLabelOther, value: $0.value as NSString)
}
contact.phoneNumbers = input.phones.map {
    CNLabeledValue(label: $0.label ?? CNLabelPhoneNumberMain, value: CNPhoneNumber(stringValue: $0.value))
}
let request = CNSaveRequest()
request.add(contact, toContainerWithIdentifier: input.containerId)
try store.execute(request)
print(contact.identifier)
