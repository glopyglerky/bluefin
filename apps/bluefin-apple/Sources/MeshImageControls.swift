#if os(iOS) && !targetEnvironment(macCatalyst)
import SwiftUI
import AVFoundation
import PhotosUI
import UniformTypeIdentifiers

struct MeshCameraScanner: UIViewControllerRepresentable {
    let scanned: (String) -> Void
    let failure: (String) -> Void
    func makeUIViewController(context: Context) -> MeshCameraController {
        MeshCameraController(scanned: scanned, failure: failure)
    }
    func updateUIViewController(_ controller: MeshCameraController, context: Context) {}
    static func dismantleUIViewController(_ controller: MeshCameraController, coordinator: ()) {
        controller.stop()
    }
}

final class MeshCameraController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let capture = AVCaptureSession()
    private let queue = DispatchQueue(label: "org.glopyglerky.bluefin.qr-camera")
    private var preview: AVCaptureVideoPreviewLayer?
    private var active = false
    private var delivered = false
    private let scanned: (String) -> Void
    private let failure: (String) -> Void
    init(scanned: @escaping (String) -> Void, failure: @escaping (String) -> Void) {
        self.scanned = scanned; self.failure = failure
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { nil }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        let layer = AVCaptureVideoPreviewLayer(session: capture)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer); preview = layer
        let cancel = UIButton(type: .system)
        cancel.setTitle("Cancel scanning", for: .normal)
        cancel.tintColor = .white
        cancel.backgroundColor = UIColor.black.withAlphaComponent(0.7)
        cancel.addTarget(self, action: #selector(cancelScan), for: .touchUpInside)
        cancel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(cancel)
        NSLayoutConstraint.activate([
            cancel.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            cancel.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            cancel.heightAnchor.constraint(greaterThanOrEqualToConstant: 44)])
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        active = true
        AVCaptureDevice.requestAccess(for: .video) { [weak self] allowed in
            DispatchQueue.main.async {
                guard let self, self.active else { return }
                if allowed { self.configure() }
                else { self.report("Camera access is off. You can paste a code or open an invitation image.") }
            }
        }
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews(); preview?.frame = view.bounds
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated); stop()
    }
    private func configure() {
        let capture = capture
        queue.async { [weak self] in
            guard let camera = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: camera) else {
                DispatchQueue.main.async { self?.report("This device has no available camera. Open an image or paste a code.") }
                return
            }
            capture.beginConfiguration()
            if capture.inputs.isEmpty {
                let output = AVCaptureMetadataOutput()
                guard capture.canAddInput(input), capture.canAddOutput(output) else {
                    capture.commitConfiguration()
                    DispatchQueue.main.async { self?.report("The camera could not start.") }
                    return
                }
                capture.addInput(input); capture.addOutput(output)
                output.setMetadataObjectsDelegate(self, queue: .main)
                guard output.availableMetadataObjectTypes.contains(.qr) else {
                    capture.commitConfiguration()
                    DispatchQueue.main.async { self?.report("QR scanning is unavailable on this camera.") }
                    return
                }
                output.metadataObjectTypes = [.qr]
            }
            capture.commitConfiguration()
            capture.startRunning()
        }
    }
    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard active, !delivered else { return }
        let codes = Set(objects.compactMap { ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue })
        guard !codes.isEmpty else { return }
        guard codes.count == 1, let value = codes.first, value.utf8.count <= 4096 else {
            report("Show exactly one Bluefin invitation QR code."); return
        }
        delivered = true; stop(); scanned(value)
    }
    @objc private func cancelScan() { stop(); dismiss(animated: true) }
    private func report(_ message: String) {
        guard active, !delivered else { return }
        delivered = true; stop(); failure(message)
    }
    func stop() {
        active = false
        let capture = capture
        queue.async { if capture.isRunning { capture.stopRunning() } }
    }
}

struct MeshPhotoPicker: UIViewControllerRepresentable {
    let selected: (Result<Data, Error>) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(selected: selected) }
    func makeUIViewController(context: Context) -> PHPickerViewController {
        var configuration = PHPickerConfiguration()
        configuration.filter = .images
        configuration.selectionLimit = 1
        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = context.coordinator
        return picker
    }
    func updateUIViewController(_ controller: PHPickerViewController, context: Context) {}
    final class Coordinator: NSObject, PHPickerViewControllerDelegate {
        let selected: (Result<Data, Error>) -> Void
        init(selected: @escaping (Result<Data, Error>) -> Void) { self.selected = selected }
        func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
            guard let provider = results.first?.itemProvider else {
                selected(.failure(CancellationError())); return
            }
            let type = provider.hasItemConformingToTypeIdentifier(UTType.png.identifier)
                ? UTType.png.identifier : UTType.jpeg.identifier
            guard provider.hasItemConformingToTypeIdentifier(type) else {
                selected(.failure(BluefinError.message("Choose a PNG or JPEG invitation image."))); return
            }
            provider.loadFileRepresentation(forTypeIdentifier: type) { [selected] url, error in
                let result: Result<Data, Error>
                if let url { result = Result { try MeshImageInput.read(url) } }
                else { result = .failure(error ?? BluefinError.message("The invitation image could not be opened.")) }
                DispatchQueue.main.async { selected(result) }
            }
        }
    }
}
#endif
