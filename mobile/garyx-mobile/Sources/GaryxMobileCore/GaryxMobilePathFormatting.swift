import Foundation

extension String {
    var garyxLastPathComponent: String {
        (self as NSString).lastPathComponent
    }

    var garyxDisambiguatedWorkspaceName: String {
        let current = (self as NSString).lastPathComponent
        let parent = ((self as NSString).deletingLastPathComponent as NSString).lastPathComponent
        guard !parent.isEmpty, parent != "/" else {
            return current.isEmpty ? self : current
        }
        return "\(parent)/\(current)"
    }
}
