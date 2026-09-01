import CoreML
import Foundation
import ImageIO
import Vision

private struct Response: Codable {
    let faces: [Face]
}

private struct Face: Codable {
    let faceIndex: Int
    let boundingBox: BoundingBox
    let faceprint: String
}

private struct BoundingBox: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

private enum HelperError: Error, CustomStringConvertible {
    case message(String)

    var description: String {
        switch self {
        case .message(let text): text
        }
    }
}

private func orientation(of url: URL) -> CGImagePropertyOrientation {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let value = properties[kCGImagePropertyOrientation] as? NSNumber else {
        return .up
    }
    return CGImagePropertyOrientation(rawValue: value.uint32Value) ?? .up
}

private func faces(url: URL) throws -> [Face] {
    let handler = VNImageRequestHandler(url: url, orientation: orientation(of: url), options: [:])
    let detection = VNDetectFaceRectanglesRequest()
    try useCPU(detection)
    try handler.perform([detection])
    let observations = (detection.results ?? []).sorted {
        if $0.boundingBox.maxY == $1.boundingBox.maxY {
            return $0.boundingBox.minX < $1.boundingBox.minX
        }
        return $0.boundingBox.maxY > $1.boundingBox.maxY
    }
    guard !observations.isEmpty else {
        throw HelperError.message("query photo must contain at least one detectable face; found 0")
    }

    return try observations.enumerated().map { offset, observation in
        let data = try faceprint(for: observation, using: handler)
        let box = observation.boundingBox
        return Face(
            faceIndex: offset + 1,
            boundingBox: BoundingBox(
                x: box.origin.x,
                y: box.origin.y,
                width: box.size.width,
                height: box.size.height
            ),
            faceprint: data.base64EncodedString()
        )
    }
}

private func faceprint(
    for observation: VNFaceObservation,
    using handler: VNImageRequestHandler
) throws -> Data {
    guard let requestType = NSClassFromString("VNCreateFaceprintRequest") as? VNRequest.Type else {
        throw HelperError.message("this macOS version does not provide the Photos faceprint runtime")
    }
    let request = requestType.init()
    let inputSelector = NSSelectorFromString("setInputFaceObservations:")
    guard request.responds(to: inputSelector) else {
        throw HelperError.message("the Photos faceprint runtime is incompatible")
    }
    request.perform(inputSelector, with: [observation])
    try useCPU(request)
    try handler.perform([request])

    let faceprintSelector = NSSelectorFromString("faceprint")
    let dataSelector = NSSelectorFromString("VNEntityIdentificationModelPrintData")
    guard let observation = request.results?.first as? NSObject,
          observation.responds(to: faceprintSelector),
          let faceprint = observation.perform(faceprintSelector)?.takeUnretainedValue() as? NSObject,
          faceprint.responds(to: dataSelector),
          let data = faceprint.perform(dataSelector)?.takeUnretainedValue() as? Data else {
        throw HelperError.message("the Photos faceprint runtime returned incompatible data")
    }
    return data
}

private func useCPU(_ request: VNRequest) throws {
    for (stage, devices) in try request.supportedComputeStageDevices {
        if let cpu = devices.first(where: { if case .cpu = $0 { return true }; return false }) {
            request.setComputeDevice(cpu, for: stage)
        }
    }
}

do {
    guard CommandLine.arguments.count == 2 else {
        throw HelperError.message("invalid Vision helper invocation")
    }
    let url = URL(fileURLWithPath: CommandLine.arguments[1]).standardizedFileURL
    let response = Response(faces: try faces(url: url))
    let data = try JSONEncoder().encode(response)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8))
    exit(1)
}
