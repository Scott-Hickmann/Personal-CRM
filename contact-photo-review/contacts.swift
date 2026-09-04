import Contacts
import CryptoKit
import AddressBook
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
    CNContactImageDataAvailableKey as CNKeyDescriptor, CNContactImageDataKey as CNKeyDescriptor,
    CNContactThumbnailImageDataKey as CNKeyDescriptor]

func hasPhoto(_ contact: CNContact) -> Bool {
    contact.imageDataAvailable || contact.imageData != nil || contact.thumbnailImageData != nil
}

func fetch(_ store: CNContactStore, id: String? = nil) throws -> [CNContact] {
    let request = CNContactFetchRequest(keysToFetch: keys)
    // Keep concrete record IDs: a unified save can propagate to linked cards.
    request.unifyResults = false
    if let id { request.predicate = CNContact.predicateForContacts(withIdentifiers: [id]) }
    var records: [CNContact] = []
    try store.enumerateContacts(with: request) { contact, _ in records.append(contact) }
    return records
}

func decodeImage(_ data: Data) throws -> CGImage {
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
        kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 2048]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
        throw Failure(message: "Cannot decode image")
    }
    return image
}

func encodeImage(_ image: CGImage) throws -> Data {
    let output = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(output, UTType.jpeg.identifier as CFString, 1, nil) else {
        throw Failure(message: "Cannot encode image")
    }
    CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
    try require(CGImageDestinationFinalize(destination), "Cannot finish image")
    return output as Data
}

func normalize(_ input: [String: String]) throws {
    let data = try Data(contentsOf: URL(fileURLWithPath: input["input"] ?? ""))
    // Keep an upright, metadata-free original so later crops use these same pixels.
    let original = try encodeImage(decodeImage(data))
    let image = try decodeImage(original)
    let rect = try detectFaceCrop(image)
    let output = try encodeImage(renderCrop(image, rect: rect))
    try original.write(to: URL(fileURLWithPath: input["original"] ?? ""), options: .atomic)
    try output.write(to: URL(fileURLWithPath: input["output"] ?? ""), options: .atomic)
    try emit(["sha256": digest(output), "original_sha256": digest(original),
              "width": image.width, "height": image.height, "crop": cropCoordinates(rect),
              "automatic": cropCoordinates(rect)])
}

func validateApproval(_ contact: CNContact, _ input: [String: String], _ data: Data) throws {
    try require(contact.identifier == input["id"], "Contact identifier changed; refresh")
    try require(identity(contact)["fingerprint"] == input["fingerprint"], "Contact details changed; refresh and review again")
    try require(!hasPhoto(contact), "Contact already has a photo; refusing to overwrite")
    try require(digest(data) == input["sha256"], "Approved image changed; review it again")
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { throw Failure(message: "Invalid photo") }
    try require(CGImageSourceGetType(source) as String? == UTType.jpeg.identifier && data.count <= 10_000_000,
                "Photo must be a normalized JPEG")
}

func mainCommand() throws {
    let command = CommandLine.arguments.dropFirst().first ?? ""
    let input = try JSONSerialization.jsonObject(with: FileHandle.standardInput.readDataToEndOfFile()) as? [String: String] ?? [:]
    if command == "normalize" { try normalize(input); return }
    if command == "recrop" { try recrop(input); return }
    try require(["list", "approve"].contains(command), "Unknown command")
    let store = CNContactStore()
    let semaphore = DispatchSemaphore(value: 0)
    var allowed = false
    store.requestAccess(for: .contacts) { granted, _ in allowed = granted; semaphore.signal() }
    semaphore.wait()
    try require(allowed, "Allow Contacts access in System Settings → Privacy & Security → Contacts, then retry.")
    if command == "list" {
        let records = try fetch(store)
        let missing = records.filter { !hasPhoto($0) }
        try emit(["contacts": missing.map(identity).sorted { $0["name"]! < $1["name"]! }, "total": records.count])
        return
    }
    guard let id = input["id"], input["fingerprint"] != nil, let file = input["image"],
          input["sha256"] != nil, let backupPath = input["backup"] else {
        throw Failure(message: "Incomplete approval")
    }
    let data = try Data(contentsOf: URL(fileURLWithPath: file))
    let records = try fetch(store, id: id)
    try require(records.count == 1 && records[0].identifier == id, "Contact disappeared or is ambiguous; refresh")
    let contact = records[0]
    try validateApproval(contact, input, data)
    guard let book = ABAddressBook.shared(),
          let person = book.record(forUniqueId: id) as? ABPerson else {
        throw Failure(message: "Contact could not be opened for a photo-only update; refresh")
    }
    try require(person.imageData() == nil, "Contact already has a photo; refusing to overwrite")
    let backup = URL(fileURLWithPath: backupPath)
    try require(!FileManager.default.fileExists(atPath: backup.path), "Backup already exists; refresh before retrying")
    try person.vCardRepresentation().write(to: backup, options: .withoutOverwriting)
    try require(person.setImageData(data), "Address Book rejected the photo")
    try book.saveAndReturnError()
    let saved = try fetch(store, id: id)
    try require(saved.count == 1 && hasPhoto(saved[0]), "Save returned but photo could not be verified; inspect Contacts before retrying")
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
