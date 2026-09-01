import CoreML
import Foundation
import ImageIO
import Vision

private struct Response: Codable {
    let faceprint: String
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

private func faceprint(url: URL) throws -> Data {
    let handler = VNImageRequestHandler(url: url, orientation: orientation(of: url), options: [:])
    let detection = VNDetectFaceRectanglesRequest()
    try useCPU(detection)
    try handler.perform([detection])
    let faces = detection.results ?? []
    guard faces.count == 1 else {
        throw HelperError.message(
            "query photo must contain exactly one detectable face; found \(faces.count)"
        )
    }

    guard let requestType = NSClassFromString("VNCreateFaceprintRequest") as? VNRequest.Type else {
        throw HelperError.message("this macOS version does not provide the Photos faceprint runtime")
    }
    let request = requestType.init()
    let inputSelector = NSSelectorFromString("setInputFaceObservations:")
    guard request.responds(to: inputSelector) else {
        throw HelperError.message("the Photos faceprint runtime is incompatible")
    }
    request.perform(inputSelector, with: faces)
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
    let response = Response(faceprint: try faceprint(url: url).base64EncodedString())
    let data = try JSONEncoder().encode(response)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8))
    exit(1)
}
