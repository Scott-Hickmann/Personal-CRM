import CoreGraphics
import Foundation
import Vision

// Vision uses a lower-left origin; CGImage cropping uses a top-left origin.
func faceCropRect(width: Int, height: Int, faces: [CGRect]) throws -> CGRect {
    try require(faces.count == 1, faces.isEmpty
        ? "No clear face detected; trying another photo"
        : "Multiple faces detected; trying a single-person photo")
    let bounds = CGRect(x: 0, y: 0, width: width, height: height)
    let box = faces[0]
    try require(box.width > 0 && box.height > 0 && CGRect(x: 0, y: 0, width: 1, height: 1).contains(box),
                "Invalid face location")
    let face = CGRect(x: box.minX * bounds.width, y: (1 - box.maxY) * bounds.height,
                      width: box.width * bounds.width, height: box.height * bounds.height)
    try require(min(face.width, face.height) >= 48, "Face is too small for a clear contact photo")
    // A tight square with room for hair, ears, and a circular Contacts display.
    let side = min(ceil(max(face.width * 1.65, face.height * 1.5)), bounds.width, bounds.height)
    let x = min(max(floor(face.midX - side / 2), 0), bounds.width - side)
    let y = min(max(floor(face.midY - face.height * 0.08 - side / 2), 0), bounds.height - side)
    let crop = CGRect(x: x, y: y, width: side, height: side)
    try require(side >= 96 && crop.contains(face), "Cannot frame this face without cutting it off")
    return crop
}

func cropFace(_ image: CGImage) throws -> CGImage {
    let request = VNDetectFaceRectanglesRequest()
    try VNImageRequestHandler(cgImage: image, orientation: .up).perform([request])
    let rect = try faceCropRect(width: image.width, height: image.height,
                                faces: (request.results ?? []).map(\.boundingBox))
    guard let cropped = image.cropping(to: rect) else { throw Failure(message: "Cannot crop face") }
    if cropped.width <= 1024 { return cropped }
    guard let context = CGContext(data: nil, width: 1024, height: 1024, bitsPerComponent: 8,
                                  bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else {
        throw Failure(message: "Cannot resize face crop")
    }
    context.interpolationQuality = .high
    context.draw(cropped, in: CGRect(x: 0, y: 0, width: 1024, height: 1024))
    guard let result = context.makeImage() else { throw Failure(message: "Cannot render face crop") }
    return result
}
