import SwiftUI

/// Branded startup loading screen shown while saved gateway settings connect
/// directly: just the centered Garyx mark on the page background.
struct GaryxStartupLoadingView: View {
    var body: some View {
        VStack(spacing: 0) {
            Spacer()

            Image("GaryxAppMark")
                .resizable()
                .scaledToFit()
                .frame(width: 116, height: 116)
                .shadow(color: Color(red: 0.10, green: 0.11, blue: 0.12).opacity(0.16), radius: 14, x: 0, y: 10)

            Spacer()
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .garyxPageBackground()
    }
}
