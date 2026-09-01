import AppKit
import Foundation
import UniformTypeIdentifiers

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("invalid photo picker invocation\n".utf8))
    exit(1)
}

let panel = NSOpenPanel()
panel.title = "Choose a photo for \(CommandLine.arguments[1])"
panel.prompt = "Choose Photo"
panel.allowedContentTypes = [.image]
panel.allowsMultipleSelection = false
panel.canChooseDirectories = false

guard panel.runModal() == .OK, let url = panel.url else {
    FileHandle.standardError.write(Data("cancelled\n".utf8))
    exit(2)
}
FileHandle.standardOutput.write(Data("\(url.path)\n".utf8))
