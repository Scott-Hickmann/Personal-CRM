import Contacts
import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct Failure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}
func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw Failure(message: message) }
}
func digest(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}
func emit(_ value: Any) throws {
    print(String(data: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), encoding: .utf8)!)
}
func identity(_ contact: CNContact) -> [String: String] {
    let name = CNContactFormatter.string(from: contact, style: .fullName) ?? contact.organizationName
    let fields = [contact.identifier, name, contact.organizationName, contact.jobTitle,
                  contact.emailAddresses.map { $0.value as String }.joined(separator: "\n"),
                  contact.phoneNumbers.map { $0.value.stringValue }.joined(separator: "\n")]
    return ["id": contact.identifier, "name": name, "organization": contact.organizationName,
            "job": contact.jobTitle, "email": contact.emailAddresses.first?.value as String? ?? "",
            "fingerprint": digest(try! JSONSerialization.data(withJSONObject: fields))]
}
let keys: [CNKeyDescriptor] = [CNContactIdentifierKey as CNKeyDescriptor,
    CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
    CNContactOrganizationNameKey as CNKeyDescriptor, CNContactJobTitleKey as CNKeyDescriptor,
    CNContactEmailAddressesKey as CNKeyDescriptor, CNContactPhoneNumbersKey as CNKeyDescriptor,
    CNContactImageDataAvailableKey as CNKeyDescriptor, CNContactImageDataKey as CNKeyDescriptor]

func fetch(_ store: CNContactStore, id: String? = nil, backup: Bool = false) throws -> [CNContact] {
    let request = CNContactFetchRequest(keysToFetch: keys + (backup ? [CNContactVCardSerialization.descriptorForRequiredKeys()] : []))
    // Keep concrete record IDs: a unified save can propagate to linked cards.
    request.unifyResults = false
    if let id { request.predicate = CNContact.predicateForContacts(withIdentifiers: [id]) }
    var records: [CNContact] = []
    try store.enumerateContacts(with: request) { contact, _ in records.append(contact) }
    return records
}

func normalize(_ input: [String: String]) throws {
    let data = try Data(contentsOf: URL(fileURLWithPath: input["input"] ?? ""))
    try require(data.count <= 10_000_000, "Image exceeds 10 MB")
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int else {
        throw Failure(message: "Unsupported or damaged image")
    }
    try require(width >= 96 && height >= 96 && width <= 20000 && height <= 20000 && width * height <= 40_000_000,
                "Image dimensions must be at least 96 px and at most 40 megapixels")
    let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 1024]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
        throw Failure(message: "Cannot decode image")
    }
    let output = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(output, UTType.jpeg.identifier as CFString, 1, nil) else {
        throw Failure(message: "Cannot encode image")
    }
    CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
    try require(CGImageDestinationFinalize(destination), "Cannot finish image")
    try (output as Data).write(to: URL(fileURLWithPath: input["output"] ?? ""), options: .atomic)
    try emit(["sha256": digest(output as Data)])
}

func prepareApproval(_ contact: CNContact, _ input: [String: String], _ data: Data) throws -> CNMutableContact {
    try require(contact.identifier == input["id"], "Contact identifier changed; refresh")
    try require(identity(contact)["fingerprint"] == input["fingerprint"], "Contact details changed; refresh and review again")
    try require(!contact.imageDataAvailable && contact.imageData == nil, "Contact already has a photo; refusing to overwrite")
    try require(digest(data) == input["sha256"], "Approved image changed; review it again")
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { throw Failure(message: "Invalid photo") }
    try require(CGImageSourceGetType(source) as String? == UTType.jpeg.identifier && data.count <= 10_000_000,
                "Photo must be a normalized JPEG")
    let mutable = contact.mutableCopy() as! CNMutableContact
    mutable.imageData = data
    return mutable
}

func mainCommand() throws {
    let command = CommandLine.arguments.dropFirst().first ?? ""
    let input = try JSONSerialization.jsonObject(with: FileHandle.standardInput.readDataToEndOfFile()) as? [String: String] ?? [:]
    if command == "normalize" { try normalize(input); return }
    try require(["list", "approve"].contains(command), "Unknown command")
    let store = CNContactStore()
    let semaphore = DispatchSemaphore(value: 0)
    var allowed = false
    store.requestAccess(for: .contacts) { granted, _ in allowed = granted; semaphore.signal() }
    semaphore.wait()
    try require(allowed, "Allow Contacts access in System Settings → Privacy & Security → Contacts, then retry.")
    if command == "list" {
        let records = try fetch(store)
        let missing = records.filter { !$0.imageDataAvailable && $0.imageData == nil }
        try emit(["contacts": missing.map(identity).sorted { $0["name"]! < $1["name"]! }, "total": records.count])
        return
    }
    guard let id = input["id"], input["fingerprint"] != nil, let file = input["image"],
          input["sha256"] != nil, let backupPath = input["backup"] else {
        throw Failure(message: "Incomplete approval")
    }
    let data = try Data(contentsOf: URL(fileURLWithPath: file))
    let records = try fetch(store, id: id, backup: true)
    try require(records.count == 1 && records[0].identifier == id, "Contact disappeared or is ambiguous; refresh")
    let contact = records[0]
    let mutable = try prepareApproval(contact, input, data)
    let backup = URL(fileURLWithPath: backupPath)
    try require(!FileManager.default.fileExists(atPath: backup.path), "Backup already exists; refresh before retrying")
    try CNContactVCardSerialization.data(with: [contact]).write(to: backup, options: .withoutOverwriting)
    let request = CNSaveRequest()
    request.update(mutable)
    try store.execute(request)
    let saved = try fetch(store, id: id)
    try require(saved.count == 1 && saved[0].imageDataAvailable, "Save returned but photo could not be verified; inspect Contacts before retrying")
    try emit(["saved": true, "id": id])
}
// The test build exercises native mutation guards using in-memory contacts only.
#if !PHOTO_REVIEW_TESTS
@main struct Entry {
    static func main() {
        do { try mainCommand() } catch {
            FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8))
            exit(1)
        }
    }
}
#endif
