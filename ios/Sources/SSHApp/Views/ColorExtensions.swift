import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

extension Color {
    public static var appBackground: Color {
        #if canImport(UIKit)
        return Color(UIColor.systemBackground)
        #elseif canImport(AppKit)
        return Color(NSColor.windowBackgroundColor)
        #else
        return Color.black
        #endif
    }

    public static var appSecondaryBackground: Color {
        #if canImport(UIKit)
        return Color(UIColor.secondarySystemBackground)
        #elseif canImport(AppKit)
        return Color(NSColor.controlBackgroundColor)
        #else
        return Color.gray.opacity(0.2)
        #endif
    }

    public static var appTertiaryBackground: Color {
        #if canImport(UIKit)
        return Color(UIColor.tertiarySystemBackground)
        #elseif canImport(AppKit)
        return Color(NSColor.underPageBackgroundColor)
        #else
        return Color.gray.opacity(0.1)
        #endif
    }
}
