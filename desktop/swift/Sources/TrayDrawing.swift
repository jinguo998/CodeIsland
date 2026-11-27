// Record the status item's original paths for the Windows taskbar rasterizer.
// Windows supplies the icon colour and pixel density.
final class DesktopTrayDrawing {
    typealias CGFloat = Double
    struct NSSize { let width, height: Double }
    struct NSRect: Encodable {
        let x, y, width, height: Double
        var midX: Double { x + width / 2 }
        var midY: Double { y + height / 2 }
    }
    struct Path: Encodable {
        let rect: NSRect
        let rx, ry: Double
        let clear: Bool
    }
    struct Mask: Encodable { let width, height: Double; let paths: [Path] }
    struct Fill {
        let fill: () -> Void
    }
    enum Blend { case sourceOver, destinationOut }

    var width = 0.0, height = 0.0
    var compositingOperation = Blend.sourceOver
    private var saved: [Blend] = []
    private var paths: [Path] = []

    func saveGraphicsState() { saved.append(compositingOperation) }
    func restoreGraphicsState() { compositingOperation = saved.removeLast() }

    func NSBezierPath(roundedRect rect: NSRect, xRadius: Double, yRadius: Double) -> Fill {
        Fill { self.paths.append(Path(rect: rect, rx: xRadius, ry: yRadius, clear: self.compositingOperation == .destinationOut)) }
    }
    func NSBezierPath(ovalIn rect: NSRect) -> Fill {
        NSBezierPath(roundedRect: rect, xRadius: rect.width / 2, yRadius: rect.height / 2)
    }

    static func mask() -> Mask {
        let drawing = DesktopTrayDrawing()
        drawing.draw()
        return Mask(width: drawing.width, height: drawing.height, paths: drawing.paths)
    }
}
