// Prints "<windowId>\t<owner>\t<title>" for on-screen windows whose owner name contains the argument.
// Run: swift tests/winid.swift prism-desktop
import CoreGraphics
import Foundation
let needle = CommandLine.arguments.count > 1 ? CommandLine.arguments[1].lowercased() : ""
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
for w in list {
    let owner = (w[kCGWindowOwnerName as String] as? String) ?? ""
    let title = (w[kCGWindowName as String] as? String) ?? ""
    let id = w[kCGWindowNumber as String] as? Int ?? 0
    let layer = w[kCGWindowLayer as String] as? Int ?? 0
    if layer == 0, owner.lowercased().contains(needle) { print("\(id)\t\(owner)\t\(title)") }
}
