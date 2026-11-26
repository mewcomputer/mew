import SwiftUI

enum PhoneAppearance: String, CaseIterable {
    case system, light, dark

    var label: String { rawValue.capitalized }
    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

enum ReviewTextSize: String, CaseIterable {
    case small, standard, large

    var label: String { rawValue.capitalized }
    var scale: CGFloat {
        switch self {
        case .small: return 11 / 13
        case .standard: return 1
        case .large: return 16 / 13
        }
    }
}

/// Local presentation preferences. These never change daemon policy or pending requests.
struct PhoneSettings: Equatable {
    var appearance: PhoneAppearance = .system
    var lightThemeId: String = MewThemes.defaultLightId
    var darkThemeId: String = MewThemes.defaultDarkId
    var haptics = true
    var completionAlerts = true
    var errorAlerts = true
    var requestAlerts = true
    var mutedMachines: Set<String> = []
    var reviewWrapLines = false
    var reviewLineNumbers = true
    var reviewTextSize: ReviewTextSize = .standard

    init() {}

    init(defaults: UserDefaults) {
        appearance = PhoneAppearance(rawValue: defaults.string(forKey: "mew.appearance") ?? "") ?? .system
        let storedLight = defaults.string(forKey: "mew.lightThemeId") ?? ""
        lightThemeId = MewThemes.lightThemes.contains(where: { $0.id == storedLight }) ? storedLight : MewThemes.defaultLightId
        let storedDark = defaults.string(forKey: "mew.darkThemeId") ?? ""
        darkThemeId = MewThemes.darkThemes.contains(where: { $0.id == storedDark }) ? storedDark : MewThemes.defaultDarkId
        haptics = defaults.object(forKey: "mew.haptics") as? Bool ?? true
        completionAlerts = defaults.object(forKey: "mew.completionAlerts") as? Bool ?? true
        errorAlerts = defaults.object(forKey: "mew.errorAlerts") as? Bool ?? true
        requestAlerts = defaults.object(forKey: "mew.requestAlerts") as? Bool ?? true
        mutedMachines = Set(defaults.stringArray(forKey: "mew.mutedMachines") ?? [])
        reviewWrapLines = defaults.object(forKey: "mew.reviewWrapLines") as? Bool ?? false
        reviewLineNumbers = defaults.object(forKey: "mew.reviewLineNumbers") as? Bool ?? true
        reviewTextSize = ReviewTextSize(rawValue: defaults.string(forKey: "mew.reviewTextSize") ?? "") ?? .standard
    }

    func save(to defaults: UserDefaults) {
        defaults.set(appearance.rawValue, forKey: "mew.appearance")
        defaults.set(lightThemeId, forKey: "mew.lightThemeId")
        defaults.set(darkThemeId, forKey: "mew.darkThemeId")
        defaults.set(haptics, forKey: "mew.haptics")
        defaults.set(completionAlerts, forKey: "mew.completionAlerts")
        defaults.set(errorAlerts, forKey: "mew.errorAlerts")
        defaults.set(requestAlerts, forKey: "mew.requestAlerts")
        defaults.set(mutedMachines.sorted(), forKey: "mew.mutedMachines")
        defaults.set(reviewWrapLines, forKey: "mew.reviewWrapLines")
        defaults.set(reviewLineNumbers, forKey: "mew.reviewLineNumbers")
        defaults.set(reviewTextSize.rawValue, forKey: "mew.reviewTextSize")
    }

    func allowsAlert(kind: String, daemon: String) -> Bool {
        guard !mutedMachines.contains(daemon) else { return false }
        switch kind {
        case "turncomplete": return completionAlerts
        case "error", "turnfailed": return errorAlerts
        case "permissionneeded", "inputneeded": return requestAlerts
        default: return false
        }
    }
}

struct PhoneFeedback: Equatable {
    enum Kind { case sent, completed, attention, failed }
    let id = UUID()
    let kind: Kind
}
