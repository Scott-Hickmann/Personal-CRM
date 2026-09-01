import Contacts
import Foundation

struct ContainerOutput: Codable {
    let id: String
    let name: String
    let type: String
}

struct LabeledValue: Codable {
    let label: String?
    let value: String
}

struct ContactOutput: Codable {
    let id: String
    let namePrefix: String
    let givenName: String
    let middleName: String
    let familyName: String
    let nameSuffix: String
    let nickname: String
    let emails: [LabeledValue]
    let phones: [LabeledValue]
    let organization: String
    let department: String
    let jobTitle: String
}

func containerType(_ type: CNContainerType) -> String {
    switch type {
    case .local: return "local"
    case .exchange: return "exchange"
    case .cardDAV: return "carddav"
    case .unassigned: return "unassigned"
    @unknown default: return "unknown"
    }
}

func requireAccess(_ store: CNContactStore) throws {
    switch CNContactStore.authorizationStatus(for: .contacts) {
    case .authorized:
        return
    case .notDetermined:
        let semaphore = DispatchSemaphore(value: 0)
        var granted = false
        var requestError: Error?
        store.requestAccess(for: .contacts) { allowed, error in
            granted = allowed
            requestError = error
            semaphore.signal()
        }
        semaphore.wait()
        if let requestError { throw requestError }
        if !granted {
            throw NSError(domain: "personal-crm", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "Contacts access was denied"])
        }
    default:
        throw NSError(domain: "personal-crm", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "Contacts access is not authorized"])
    }
}

func encode<T: Encodable>(_ value: T) throws {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    FileHandle.standardOutput.write(try encoder.encode(value))
}

do {
    let arguments = CommandLine.arguments
    guard arguments.count >= 2 else {
        throw NSError(domain: "personal-crm", code: 2,
                      userInfo: [NSLocalizedDescriptionKey: "expected containers or export"])
    }
    let store = CNContactStore()
    try requireAccess(store)

    if arguments[1] == "containers" {
        let output = try store.containers(matching: nil).map {
            ContainerOutput(id: $0.identifier, name: $0.name, type: containerType($0.type))
        }.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        try encode(output)
    } else if arguments[1] == "export", arguments.count == 3 {
        let containerID = arguments[2]
        let containers = try store.containers(
            matching: CNContainer.predicateForContainers(withIdentifiers: [containerID])
        )
        guard containers.count == 1 else {
            throw NSError(domain: "personal-crm", code: 3,
                          userInfo: [NSLocalizedDescriptionKey: "configured contact container was not found"])
        }
        let keys = [
            CNContactIdentifierKey, CNContactNamePrefixKey, CNContactGivenNameKey,
            CNContactMiddleNameKey, CNContactFamilyNameKey, CNContactNameSuffixKey,
            CNContactNicknameKey, CNContactEmailAddressesKey, CNContactPhoneNumbersKey,
            CNContactOrganizationNameKey, CNContactDepartmentNameKey, CNContactJobTitleKey,
        ] as [CNKeyDescriptor]
        let request = CNContactFetchRequest(keysToFetch: keys)
        request.predicate = CNContact.predicateForContactsInContainer(withIdentifier: containerID)
        request.unifyResults = false
        var output: [ContactOutput] = []
        try store.enumerateContacts(with: request) { contact, _ in
            output.append(ContactOutput(
                id: contact.identifier,
                namePrefix: contact.namePrefix,
                givenName: contact.givenName,
                middleName: contact.middleName,
                familyName: contact.familyName,
                nameSuffix: contact.nameSuffix,
                nickname: contact.nickname,
                emails: contact.emailAddresses.map {
                    LabeledValue(label: $0.label, value: $0.value as String)
                },
                phones: contact.phoneNumbers.map {
                    LabeledValue(label: $0.label, value: $0.value.stringValue)
                },
                organization: contact.organizationName,
                department: contact.departmentName,
                jobTitle: contact.jobTitle
            ))
        }
        try encode(output.sorted { $0.id < $1.id })
    } else {
        throw NSError(domain: "personal-crm", code: 2,
                      userInfo: [NSLocalizedDescriptionKey: "invalid Contacts helper arguments"])
    }
} catch {
    FileHandle.standardError.write(Data("\(error.localizedDescription)\n".utf8))
    exit(1)
}
