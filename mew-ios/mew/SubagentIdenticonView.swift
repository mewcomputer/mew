import SwiftUI

let subagentIdenticonPaletteHex = [
    "#3b82f6",
    "#10b981",
    "#f59e0b",
    "#8b5cf6",
    "#ec4899",
    "#06b6d4",
    "#f97316",
    "#6366f1",
]

private let paletteRGB: [(Double, Double, Double)] = [
    (0x3b / 255.0, 0x82 / 255.0, 0xf6 / 255.0),
    (0x10 / 255.0, 0xb9 / 255.0, 0x81 / 255.0),
    (0xf5 / 255.0, 0x9e / 255.0, 0x0b / 255.0),
    (0x8b / 255.0, 0x5c / 255.0, 0xf6 / 255.0),
    (0xec / 255.0, 0x48 / 255.0, 0x99 / 255.0),
    (0x06 / 255.0, 0xb6 / 255.0, 0xd4 / 255.0),
    (0xf9 / 255.0, 0x73 / 255.0, 0x16 / 255.0),
    (0x63 / 255.0, 0x66 / 255.0, 0xf1 / 255.0),
]

func hashSeedFnv1a(_ seed: String) -> UInt32 {
    var hash: UInt32 = 0x811c9dc5
    for byte in seed.utf8 {
        hash ^= UInt32(byte)
        hash = hash &* 0x01000193
    }
    return hash
}

func identiconColorHex(seed: String) -> String {
    let hash = hashSeedFnv1a(seed)
    return subagentIdenticonPaletteHex[Int(hash & 0x7)]
}

func identiconColor(seed: String) -> Color {
    let hash = hashSeedFnv1a(seed)
    let (r, g, b) = paletteRGB[Int(hash & 0x7)]
    return Color(red: r, green: g, blue: b)
}

/// Builds a 5x5 horizontally symmetric matrix from the deterministic FNV-1a hash.
func identiconCells(seed: String) -> [[Bool]] {
    let gridSize = 5
    let halfColumns = 3
    let hash = hashSeedFnv1a(seed)
    var pattern = (hash >> 3) & 0x7fff
    if pattern == 0 {
        // Keep zero-bit hashes visibly non-empty at the center pixel.
        pattern = 1 << (2 * halfColumns + 2)
    }
    return (0..<gridSize).map { row in
        var line = Array(repeating: false, count: gridSize)
        for col in 0..<halfColumns {
            let bitIndex = row * halfColumns + col
            let filled = ((pattern >> bitIndex) & 1) == 1
            line[col] = filled
            line[gridSize - 1 - col] = filled
        }
        return line
    }
}

struct SubagentIdenticonView: View {
    let seed: String
    var size: CGFloat = 18

    var body: some View {
        let color = identiconColor(seed: seed)
        let cells = identiconCells(seed: seed)

        Canvas { context, canvasSize in
            let cellW = canvasSize.width / 7.0
            let cellH = canvasSize.height / 7.0
            context.fill(
                Path(CGRect(origin: .zero, size: canvasSize)),
                with: .color(color.opacity(0.16))
            )
            for (rowIndex, row) in cells.enumerated() {
                for (colIndex, filled) in row.enumerated() where filled {
                    let rect = CGRect(
                        x: CGFloat(colIndex + 1) * cellW,
                        y: CGFloat(rowIndex + 1) * cellH,
                        width: cellW,
                        height: cellH
                    )
                    context.fill(Path(rect), with: .color(color))
                }
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: max(2, size * 0.18), style: .continuous))
        .accessibilityHidden(true)
    }
}
