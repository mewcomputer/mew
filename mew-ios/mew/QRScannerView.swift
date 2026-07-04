import SwiftUI
import AVFoundation

/// A SwiftUI camera view that scans for QR codes and other machine-readable
/// payloads. When a code is found, `onScan` is called with the string value.
struct QRScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void

    func makeUIViewController(context: Context) -> QRScannerViewController {
        let vc = QRScannerViewController()
        vc.onScan = onScan
        return vc
    }

    func updateUIViewController(_ vc: QRScannerViewController, context: Context) {}
}

final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let captureSession = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var hasScanned = false
    var onScan: ((String) -> Void)?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        setupCamera()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if !captureSession.isRunning {
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.captureSession.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if captureSession.isRunning {
            captureSession.stopRunning()
        }
    }

    private func setupCamera() {
        guard let device = AVCaptureDevice.default(for: .video) else {
            showError("Camera not available")
            return
        }

        do {
            let input = try AVCaptureDeviceInput(device: device)
            if captureSession.canAddInput(input) {
                captureSession.addInput(input)
            }
        } catch {
            showError("Cannot access camera: \(error.localizedDescription)")
            return
        }

        let output = AVCaptureMetadataOutput()
        if captureSession.canAddOutput(output) {
            captureSession.addOutput(output)
            output.metadataObjectTypes = [.qr]
            output.setMetadataObjectsDelegate(self, queue: .main)
        }

        let previewLayer = AVCaptureVideoPreviewLayer(session: captureSession)
        previewLayer.videoGravity = .resizeAspectFill
        previewLayer.frame = view.bounds
        view.layer.addSublayer(previewLayer)
        self.previewLayer = previewLayer

        // Add a reticle overlay
        addReticleOverlay()
    }

    private func addReticleOverlay() {
        let reticleView = UIView()
        reticleView.layer.borderColor = UIColor.white.cgColor
        reticleView.layer.borderWidth = 2
        reticleView.layer.cornerRadius = 12
        reticleView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(reticleView)

        NSLayoutConstraint.activate([
            reticleView.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            reticleView.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            reticleView.widthAnchor.constraint(equalTo: view.widthAnchor, multiplier: 0.7),
            reticleView.heightAnchor.constraint(equalTo: reticleView.widthAnchor),
        ])
    }

    private func showError(_ message: String) {
        DispatchQueue.main.async { [weak self] in
            let label = UILabel()
            label.text = message
            label.textColor = .white
            label.textAlignment = .center
            label.numberOfLines = 0
            label.font = .systemFont(ofSize: 16, weight: .medium)
            label.translatesAutoresizingMaskIntoConstraints = false
            self?.view.addSubview(label)
            NSLayoutConstraint.activate([
                label.centerXAnchor.constraint(equalTo: self!.view.centerXAnchor),
                label.centerYAnchor.constraint(equalTo: self!.view.centerYAnchor),
                label.leadingAnchor.constraint(equalTo: self!.view.leadingAnchor, constant: 32),
                label.trailingAnchor.constraint(equalTo: self!.view.trailingAnchor, constant: -32),
            ])
        }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !hasScanned,
              let obj = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let payload = obj.stringValue
        else { return }

        hasScanned = true
        // Haptic feedback
        let generator = UIImpactFeedbackGenerator(style: .medium)
        generator.impactOccurred()

        onScan?(payload)
    }

    func reset() {
        hasScanned = false
    }
}
