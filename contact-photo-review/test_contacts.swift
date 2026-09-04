import Contacts
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

@main struct NativeTests {
    static func main() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let context = CGContext(data: nil, width: 1500, height: 1200, bitsPerComponent: 8,
                                bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                                bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(CGColor(red: 0.3, green: 0.6, blue: 0.5, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 1500, height: 1200))
        let raw = NSMutableData()
        let destination = CGImageDestinationCreateWithData(raw, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyExifDictionary: ["UserComment": "private metadata"]] as CFDictionary)
        precondition(CGImageDestinationFinalize(destination))
        let original = directory.appendingPathComponent("original.jpg")
        let normalized = directory.appendingPathComponent("normalized.jpg")
        try (raw as Data).write(to: original)
        try normalize(["input": original.path, "output": normalized.path])
        let photo = try Data(contentsOf: normalized)
        let source = CGImageSourceCreateWithData(photo as CFData, nil)!
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)! as NSDictionary
        precondition(properties[kCGImagePropertyPixelWidth] as? Int == 1024)
        precondition(!(String(describing: properties).contains("private metadata")))

        let contact = CNMutableContact()
        contact.givenName = "Test"
        contact.familyName = "Person"
        contact.organizationName = "Original Organization"
        contact.note = "Must be preserved"
        contact.emailAddresses = [CNLabeledValue(label: CNLabelWork, value: "test@example.com" as NSString)]
        let input = ["id": contact.identifier, "fingerprint": identity(contact)["fingerprint"]!, "sha256": digest(photo)]
        let changed = try prepareApproval(contact, input, photo)
        precondition(changed.imageData == photo && contact.imageData == nil)
        precondition(changed.note == contact.note && changed.emailAddresses == contact.emailAddresses)
        precondition(changed.organizationName == contact.organizationName && changed.identifier == contact.identifier)

        func rejected(_ candidate: CNContact, _ input: [String: String], _ data: Data) throws {
            do {
                _ = try prepareApproval(candidate, input, data)
                throw NSError(domain: "Test unexpectedly accepted unsafe save", code: 1)
            } catch is Failure { /* Expected guard rejection. */ }
        }
        try rejected(changed, input, photo) // Existing photo.
        contact.organizationName = "Edited since review"
        try rejected(contact, input, photo)
        contact.organizationName = "Original Organization"
        try rejected(contact, input.merging(["id": "another-contact"]) { _, new in new }, photo)
        try rejected(contact, input, Data("tampered".utf8))
        let invalid = Data("not a JPEG".utf8)
        try rejected(contact, input.merging(["sha256": digest(invalid)]) { _, new in new }, invalid)
        print("Native tests passed: resize, metadata removal, preserved fields, existing photo, stale identity, wrong ID, tampering, invalid image.")
    }
}
