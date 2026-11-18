// The design tokens of DESIGN.md as Swift: ink on warm paper in light and dark, course hues as the only colour,
// SF Pro for the interface, New York for topic names and reading text, a light tabular clock.
import AppKit
import SwiftUI

extension Color {
    /// A colour with separate light and dark values that follows the window's appearance.
    init(light: NSColor, dark: NSColor) {
        self.init(nsColor: NSColor(name: nil) { $0.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light })
    }
    init(hex: UInt32, alpha: Double = 1) {
        self.init(.sRGB, red: Double(hex >> 16 & 0xFF) / 255, green: Double(hex >> 8 & 0xFF) / 255, blue: Double(hex & 0xFF) / 255, opacity: alpha)
    }
}

private func ns(_ hex: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

enum Ink {
    static let paper = Color(light: ns(0xFBFAF8), dark: ns(0x161615))
    static let panel = Color(light: ns(0xF3F2EF), dark: ns(0x1E1E1C))
    static let raised = Color(light: ns(0xFFFFFF), dark: ns(0x252523))
    static let ink = Color(light: ns(0x1C1B19), dark: ns(0xEEEDEA))
    static let ink2 = Color(light: ns(0x5F5D58), dark: ns(0xA5A39D))
    static let ink3 = Color(light: ns(0x8C8A84), dark: ns(0x77756F))
    static let line = Color(light: ns(0x1C1B19, 0.10), dark: ns(0xFFFFFA, 0.09))
    static let line2 = Color(light: ns(0x1C1B19, 0.18), dark: ns(0xFFFFFA, 0.16))
    static let hover = Color(light: ns(0x1C1B19, 0.045), dark: ns(0xFFFFFA, 0.05))
    static let sel = Color(light: ns(0x1C1B19, 0.075), dark: ns(0xFFFFFA, 0.09))
    static let due = Color(light: ns(0xB4331E), dark: ns(0xF08A70))
    static let ok = Color(light: ns(0x2F7A4B), dark: ns(0x6CC48E))
}

/// Course hues: oklch at a fixed lightness and chroma per appearance, so every course reads at the same weight.
enum Hue {
    /// The palette a course gets by its colour index (0…7).
    static let palette: [Double] = [30, 245, 145, 75, 300, 195, 350, 110]
    /// The hues offered when picking a colour.
    static let swatches: [Double] = [25, 50, 75, 100, 130, 155, 180, 200, 225, 250, 275, 300, 325, 350]

    static func color(_ hue: Double) -> Color {
        Color(light: oklch(0.58, 0.13, hue), dark: oklch(0.72, 0.11, hue))
    }

    /// oklch to sRGB (Björn Ottosson's conversion), clamped into gamut.
    static func oklch(_ l: Double, _ c: Double, _ h: Double) -> NSColor {
        let a = c * cos(h * .pi / 180), b = c * sin(h * .pi / 180)
        let l_ = l + 0.3963377774 * a + 0.2158037573 * b
        let m_ = l - 0.1055613458 * a - 0.0638541728 * b
        let s_ = l - 0.0894841775 * a - 1.2914855480 * b
        let (L, M, S) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_)
        let lin = [4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S,
                   -1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S,
                   -0.0041960863 * L - 0.7034186147 * M + 1.7076147010 * S]
        let g = lin.map { v -> CGFloat in
            let x = min(1, max(0, v))
            return CGFloat(x <= 0.0031308 ? 12.92 * x : 1.055 * pow(x, 1 / 2.4) - 0.055)
        }
        return NSColor(srgbRed: g[0], green: g[1], blue: g[2], alpha: 1)
    }
}

/// A colour picked for a course, section or folder: a hue, neutral, or a custom colour.
enum Tint: Codable, Hashable {
    case hue(Double), neutral, custom(String) // custom: "#rrggbb"

    var color: Color {
        switch self {
        case .hue(let h): Hue.color(h)
        case .neutral: Ink.ink3
        case .custom(let hex): Color(hex: UInt32(hex.dropFirst(), radix: 16) ?? 0)
        }
    }
}

enum Typeset {
    static let clock = Font.system(size: 88, weight: .ultraLight).monospacedDigit()
    static let display = Font.system(size: 30, weight: .medium, design: .serif)
    static let title = Font.system(size: 20, weight: .semibold)
    static let heading = Font.system(size: 13, weight: .medium)
    static let topic = Font.system(size: 14, design: .serif)
    static let body = Font.system(size: 13)
    static let small = Font.system(size: 12)
    static let caption = Font.system(size: 11, weight: .medium)
    static func serif(_ size: CGFloat, weight: Font.Weight = .regular) -> Font { .system(size: size, weight: weight, design: .serif) }
}

/// Hover and press motion per DESIGN.md: 120ms ease-out, none when Reduce Motion is on.
extension Animation {
    static let quick = Animation.timingCurve(0.22, 1, 0.36, 1, duration: 0.12)
}

// MARK: - Small shared pieces

/// A section heading: medium 13pt with an optional action on the right.
struct SectionHead<Trailing: View>: View {
    let title: String
    @ViewBuilder var trailing: Trailing
    init(_ title: String, @ViewBuilder trailing: () -> Trailing = { EmptyView() }) {
        self.title = title
        self.trailing = trailing()
    }
    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title).font(Typeset.heading).foregroundStyle(Ink.ink)
            Spacer()
            trailing.font(Typeset.small)
        }
    }
}

/// Mastery as a thin bar with its number.
struct MasteryBar: View {
    let value: Int
    var overdue = false
    var body: some View {
        HStack(spacing: 6) {
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(.quaternary)
                    Capsule().fill(overdue ? AnyShapeStyle(Ink.due) : AnyShapeStyle(.secondary)).frame(width: g.size.width * CGFloat(min(100, max(0, value))) / 100)
                }
            }
            .frame(width: 56, height: 4)
            Text("\(value)").font(Typeset.small.monospacedDigit()).foregroundStyle(.secondary).frame(minWidth: 22, alignment: .trailing)
        }
        .help("Mastery \(value) of 100")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Mastery \(value) of 100")
    }
}

struct StatusChip: View {
    let status: Status
    var body: some View {
        Text(status.label)
            .font(Typeset.caption)
            .foregroundStyle(status == .review ? AnyShapeStyle(Ink.due) : AnyShapeStyle(.secondary))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: 4).fill(status == .review ? AnyShapeStyle(Ink.due.opacity(0.1)) : AnyShapeStyle(.quaternary)))
    }
}

/// A course's dot.
struct Dot: View {
    let color: Color
    var size: CGFloat = 8
    var body: some View { Circle().fill(color).frame(width: size, height: size) }
}

/// The lecture file a topic came from, shown small next to it.
struct DeckLabel: View {
    let doc: Doc?
    var body: some View {
        if let doc, !doc.deckName.isEmpty {
            Label(doc.deckName, systemImage: "doc")
                .labelStyle(.titleAndIcon)
                .font(Typeset.small)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .help("Lecture file: \(doc.fileName.isEmpty ? doc.title : doc.fileName)\nLesson: \(doc.title)")
        }
    }
}

/// A key shown next to its action.
struct KeyHint: View {
    let key: String
    var body: some View {
        // Follows the button's own text colour, so it reads on ink and on paper alike.
        Text(key)
            .font(.system(size: 10, weight: .medium))
            .opacity(0.7)
            .padding(.horizontal, 4).padding(.vertical, 1)
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(.tertiary, lineWidth: 0.5))
    }
}

/// A figure with its caption, as in the stat lines.
struct Stat: View {
    let value: String
    let label: String
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(value).font(.system(size: 20, weight: .semibold).monospacedDigit()).foregroundStyle(Ink.ink)
            Text(label).font(Typeset.small).foregroundStyle(Ink.ink2)
        }
    }
}

/// The page column: max 1120pt wide with 32pt gutters, scrolling.
struct Page<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 32) { content }
                .frame(maxWidth: 1120, alignment: .leading)
                .padding(32)
                .frame(maxWidth: .infinity)
        }
        .background(Ink.paper)
    }
}

/// An empty state: a title, a sentence or two, and its actions.
struct EmptyState<Actions: View>: View {
    let title: String
    let message: String
    @ViewBuilder var actions: Actions
    var body: some View {
        VStack(spacing: 12) {
            Text(title).font(Typeset.title)
            Text(message).font(Typeset.body).foregroundStyle(Ink.ink2).multilineTextAlignment(.center).frame(maxWidth: 460)
            actions
        }
        .padding(48)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Ink.paper)
    }
}

/// The one size table for every control with text in it (buttons, fields, menus), so text sits at the same height
/// in all of them: small 22pt (inspector, Settings), regular 28pt (pages and sheets), large 36pt (Today and a session).
struct ControlMetrics {
    let height: CGFloat, pad: CGFloat, font: CGFloat
    init(_ size: ControlSize) {
        switch size {
        case .mini, .small: (height, pad, font) = (22, 8, 12)
        case .large, .extraLarge: (height, pad, font) = (36, 16, 13)
        default: (height, pad, font) = (28, 12, 13)
        }
    }
    /// Top and bottom inset that centres one line of the control's font in its height; extra lines grow the box.
    var inset: CGFloat { (height - NSLayoutManager().defaultLineHeight(for: .systemFont(ofSize: font))) / 2 }
}

/// A button's icon and title, centred on each other: a tall symbol cannot push the title off the button's middle.
private struct ControlLabel: LabelStyle {
    let font: CGFloat
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 6) {
            configuration.icon.frame(height: font)
            configuration.title
        }
    }
}

/// The app's buttons, all one shape: ink-filled primary, outlined secondary, sized by `ControlMetrics`.
struct ActionButtonStyle: ButtonStyle {
    var prominent: Bool
    /// A fixed size, for forms whose other controls stay regular (Settings).
    var size: ControlSize?
    func makeBody(configuration: Configuration) -> some View { Face(configuration: configuration, prominent: prominent, fixed: size) }

    private struct Face: View {
        let configuration: Configuration
        let prominent: Bool
        let fixed: ControlSize?
        @Environment(\.isEnabled) private var enabled
        @Environment(\.controlSize) private var size
        @State private var hover = false

        private var destructive: Bool { configuration.role == .destructive }
        private var fill: Color { prominent ? (destructive ? Ink.due : Ink.ink) : configuration.isPressed ? Ink.sel : hover ? Ink.hover : Ink.raised }

        var body: some View {
            let m = ControlMetrics(fixed ?? size), shape = RoundedRectangle(cornerRadius: 6)
            configuration.label
                .labelStyle(ControlLabel(font: m.font))
                .lineLimit(1)
                .fixedSize()
                .font(.system(size: m.font, weight: prominent ? .medium : .regular))
                .foregroundStyle(prominent ? Ink.paper : destructive ? Ink.due : Ink.ink)
                .padding(.horizontal, m.pad)
                .frame(minHeight: m.height)
                .background(shape.fill(fill.opacity(prominent && configuration.isPressed ? 0.8 : 1)))
                .overlay { if !prominent { shape.strokeBorder(Ink.line2, lineWidth: 1) } }
                .opacity(enabled ? 1 : 0.4)
                .contentShape(shape)
                .onHover { hover = $0 }
                .animation(.quick, value: hover)
        }
    }
}

extension ButtonStyle where Self == ActionButtonStyle {
    static var primary: ActionButtonStyle { .init(prominent: true) }
    static var secondary: ActionButtonStyle { .init(prominent: false) }
}

/// A text field in the buttons' shape and size, so a field and the button beside it line up. Apply it to a field
/// with its label hidden; `size` fixes the size where other controls stay regular (Settings).
struct FieldChrome: ViewModifier {
    var size: ControlSize?
    @Environment(\.controlSize) private var envSize
    @Environment(\.isEnabled) private var enabled
    @FocusState private var focused: Bool

    func body(content: Content) -> some View {
        let m = ControlMetrics(size ?? envSize), shape = RoundedRectangle(cornerRadius: 6)
        content
            .textFieldStyle(.plain)
            .font(.system(size: m.font))
            .focused($focused)
            .padding(.horizontal, m.pad - 2)
            .padding(.vertical, m.inset)
            .frame(minHeight: m.height)
            .background(shape.fill(Ink.raised))
            .overlay { shape.strokeBorder(focused ? Ink.ink3 : Ink.line2, lineWidth: focused ? 1.5 : 1) }
            .opacity(enabled ? 1 : 0.5)
            .animation(.quick, value: focused)
    }
}

extension View {
    func field(_ size: ControlSize? = nil) -> some View { modifier(FieldChrome(size: size)) }
}

/// A short inline choice (a unit, a goal kind) as a secondary button with a menu, so it matches the field and buttons
/// in its row; a system pop-up has its own height.
struct MenuChoice<Value: Hashable>: View {
    @Binding var selection: Value
    let options: [(label: String, value: Value)]
    var body: some View {
        Menu {
            ForEach(options, id: \.value) { o in
                Button { selection = o.value } label: { if o.value == selection { Label(o.label, systemImage: "checkmark") } else { Text(o.label) } }
            }
        } label: {
            HStack(spacing: 4) {
                Text(options.first { $0.value == selection }?.label ?? "")
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold)).foregroundStyle(Ink.ink3)
            }
        }
        .menuStyle(.button)
        .menuIndicator(.hidden)
        .buttonStyle(.secondary)
        .fixedSize()
    }
}

/// A date as a secondary button that opens a calendar, so it lines up with the buttons beside it.
struct DateField: View {
    @Binding var date: Date
    @State private var open = false
    var body: some View {
        Button { open = true } label: {
            HStack(spacing: 6) {
                Text(date.formatted(.dateTime.month(.abbreviated).day().year())).monospacedDigit()
                Image(systemName: "calendar").foregroundStyle(Ink.ink3)
            }
        }
        .buttonStyle(.secondary)
        .popover(isPresented: $open, arrowEdge: .bottom) {
            DatePicker("Date", selection: $date, displayedComponents: .date)
                .datePickerStyle(.graphical)
                .labelsHidden()
                .controlSize(.regular)
                .padding(12)
        }
    }
}

/// A checkbox drawn flat: a hairline square that fills with ink when on, in place of the system's bevelled box.
struct FlatCheck: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        Button { configuration.isOn.toggle() } label: {
            HStack(spacing: 8) {
                Mark(on: configuration.isOn, round: false)
                configuration.label
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityValue(configuration.isOn ? "on" : "off")
    }
}

extension ToggleStyle where Self == FlatCheck {
    static var check: FlatCheck { .init() }
}

/// The flat mark of a checkbox (square, a tick) or a radio choice (round, a dot).
private struct Mark: View {
    let on: Bool
    let round: Bool
    @Environment(\.isEnabled) private var enabled
    var body: some View {
        let shape = RoundedRectangle(cornerRadius: round ? 7 : 3.5)
        ZStack {
            shape.fill(on ? Ink.ink : Ink.raised)
            if !on { shape.strokeBorder(Ink.line2, lineWidth: 1) }
            if on {
                if round { Circle().fill(Ink.paper).frame(width: 5, height: 5) }
                else { Image(systemName: "checkmark").font(.system(size: 8, weight: .bold)).foregroundStyle(Ink.paper) }
            }
        }
        .frame(width: 14, height: 14)
        .opacity(enabled ? 1 : 0.5)
        .animation(.quick, value: on)
    }
}

/// A two-or-three-way switch drawn flat: hairline track, the chosen part raised paper with a hairline, no shadow.
struct Segments<Value: Hashable>: View {
    @Binding var selection: Value
    let options: [(label: String, value: Value)]
    var body: some View {
        HStack(spacing: 2) {
            ForEach(options, id: \.value) { o in
                let on = o.value == selection
                Button { selection = o.value } label: {
                    Text(o.label)
                        .font(.system(size: 13))
                        .foregroundStyle(on ? Ink.ink : Ink.ink2)
                        .frame(maxWidth: .infinity, minHeight: 24)
                        .background {
                            if on { RoundedRectangle(cornerRadius: 5).fill(Ink.raised).overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Ink.line2, lineWidth: 1)) }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 6).fill(Ink.hover))
        .animation(.quick, value: selection)
    }
}

/// A list of single choices with flat round marks, in place of the system's bevelled radio group.
struct ChoiceList<Value: Hashable, Row: View>: View {
    @Binding var selection: Value?
    let values: [Value]
    var spacing: CGFloat = 8
    var isDisabled: (Value) -> Bool = { _ in false }
    @ViewBuilder let row: (Value) -> Row
    var body: some View {
        VStack(alignment: .leading, spacing: spacing) {
            ForEach(values, id: \.self) { v in
                Button { selection = v } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Mark(on: selection == v, round: true).alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                        row(v).frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(isDisabled(v))
                .accessibilityAddTraits(selection == v ? .isSelected : [])
            }
        }
    }
}

/// The third tier: a text or icon action in ink-2 that darkens on hover, in place of a blue link or a borderless button.
struct QuietLink: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { Hovering(configuration: configuration) }
    private struct Hovering: View {
        let configuration: Configuration
        @State private var hover = false
        var body: some View {
            configuration.label
                .foregroundStyle(configuration.role == .destructive ? Ink.due : hover || configuration.isPressed ? Ink.ink : Ink.ink2)
                .underline(hover)
                .contentShape(Rectangle())
                .onHover { hover = $0 }
        }
    }
}

extension ButtonStyle where Self == QuietLink {
    static var quiet: QuietLink { .init() }
}
