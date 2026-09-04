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
        context.setFillColor(CGColor(red: 0.8, green: 0.2, blue: 0.1, alpha: 1))
        context.fill(CGRect(x: 750, y: 0, width: 750, height: 1200))
        let raw = NSMutableData()
        let destination = CGImageDestinationCreateWithData(raw, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyExifDictionary: ["UserComment": "private metadata"]] as CFDictionary)
        precondition(CGImageDestinationFinalize(destination))
        let original = directory.appendingPathComponent("original.jpg")
        try (raw as Data).write(to: original)
        let decoded = try decodeImage(Data(contentsOf: original))
        let crop = try faceCropRect(width: decoded.width, height: decoded.height,
                                   faces: [CGRect(x: 0.4, y: 0.5, width: 0.2, height: 0.25)])
        let photo = try encodeImage(decoded.cropping(to: crop)!)
        let source = CGImageSourceCreateWithData(photo as CFData, nil)!
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)! as NSDictionary
        precondition(properties[kCGImagePropertyPixelWidth] as? Int == 495)
        precondition(properties[kCGImagePropertyPixelHeight] as? Int == 495)
        precondition(!(String(describing: properties).contains("private metadata")))
        // The face is near the top; a missing Vision-to-CGImage Y flip lands at the bottom.
        precondition(crop.minY == 178 && crop.minX == 502)
        for box in [CGRect(x: 0, y: 0, width: 0.2, height: 0.25),
                    CGRect(x: 0.8, y: 0.75, width: 0.2, height: 0.25)] {
            let edge = try faceCropRect(width: 1500, height: 1200, faces: [box])
            precondition(CGRect(x: 0, y: 0, width: 1500, height: 1200).contains(edge))
            precondition(edge.width == edge.height)
        }
        for faces in [[], [CGRect(x: 0.1, y: 0.1, width: 0.2, height: 0.2),
                            CGRect(x: 0.6, y: 0.5, width: 0.2, height: 0.2)],
                      [CGRect(x: 0.2, y: 0.2, width: 0.01, height: 0.01)]] {
            do {
                _ = try faceCropRect(width: 1500, height: 1200, faces: faces)
                preconditionFailure("Ambiguous or tiny face must not be cropped")
            } catch is Failure { }
        }
        let originalHash = digest(raw as Data)
        let adjusted = directory.appendingPathComponent("adjusted.jpg")
        let cropInput = ["input": original.path, "output": adjusted.path, "original_sha256": originalHash,
                         "x": "0", "y": "0", "size": "300"]
        try recrop(cropInput)
        let firstCrop = try Data(contentsOf: adjusted)
        try recrop(cropInput.merging(["x": "900"]) { _, new in new })
        let secondCrop = try Data(contentsOf: adjusted)
        precondition(firstCrop != secondCrop, "Moving across the original must change the saved pixels")
        let adjustedImage = try decodeImage(secondCrop)
        let unchangedOriginal = try Data(contentsOf: original)
        precondition(adjustedImage.width == 300)
        precondition(unchangedOriginal == raw as Data, "Recropping must preserve the original")
        for change in [["x": "-1"], ["size": "95"], ["x": "1499"], ["x": "1.5"], ["original_sha256": "changed"]] {
            do {
                try recrop(cropInput.merging(change) { _, new in new })
                preconditionFailure("Invalid manual crop must fail")
            } catch is Failure { }
        }

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
        print("Native tests passed: square face framing, coordinate conversion, edge clamping, ambiguous/tiny face rejection, metadata removal, and save safeguards.")
    }
}
