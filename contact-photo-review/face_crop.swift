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

func detectFaceCrop(_ image: CGImage) throws -> CGRect {
    let request = VNDetectFaceRectanglesRequest()
    try VNImageRequestHandler(cgImage: image, orientation: .up).perform([request])
    return try faceCropRect(width: image.width, height: image.height,
                           faces: (request.results ?? []).map(\.boundingBox))
}

func renderCrop(_ image: CGImage, rect: CGRect) throws -> CGImage {
    try require(rect.width >= 96 && rect.width == rect.height && rect == rect.integral
        && CGRect(x: 0, y: 0, width: image.width, height: image.height).contains(rect),
        "Choose a square of at least 96 pixels inside the original photo")
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

func cropCoordinates(_ rect: CGRect) -> [String: Int] {
    ["x": Int(rect.minX), "y": Int(rect.minY), "size": Int(rect.width)]
}

func recrop(_ input: [String: String]) throws {
    let data = try Data(contentsOf: URL(fileURLWithPath: input["input"] ?? ""))
    try require(digest(data) == input["original_sha256"], "Original photo changed; reload before cropping")
    guard let x = Int(input["x"] ?? ""), let y = Int(input["y"] ?? ""), let size = Int(input["size"] ?? "") else {
        throw Failure(message: "Invalid crop coordinates")
    }
    let rect = CGRect(x: x, y: y, width: size, height: size)
    let output = try encodeImage(renderCrop(decodeImage(data), rect: rect))
    try output.write(to: URL(fileURLWithPath: input["output"] ?? ""), options: .atomic)
    try emit(["sha256": digest(output)])
}
