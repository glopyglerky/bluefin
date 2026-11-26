import XCTest
#if os(tvOS)
@testable import BluefinTV
#else
@testable import Bluefin
#endif

@MainActor private final class ContinuityAudioFake: KaraokeContinuityAudio {
    var permission = true
    var pending: CheckedContinuation<Bool, Never>?
    var suspend = false
    var failStart = false
    var failRestore = false
    var starts = 0
    var stops = 0
    var active = false
    var gain: Float = 0
    func requestPermission() async -> Bool {
        if suspend { return await withCheckedContinuation { pending = $0 } }
        return permission
    }
    func start(gain: Float) throws {
        starts += 1; active = true; self.gain = gain
        if failStart { throw Failure.injected }
    }
    func setGain(_ gain: Float) { self.gain = gain }
    func stop() throws {
        stops += 1; active = false
        if failRestore { throw Failure.injected }
    }
    enum Failure: Error { case injected }
}

final class KaraokeContinuityTests: XCTestCase {
    @MainActor func testSelectionRequiresSeparateConsentAndCapsGain() async {
        let audio = ContinuityAudioFake(), model = KaraokeContinuityLifecycle(audio: ContinuityAudioFake())
        await model.enable()
        XCTAssertEqual(model.state, .off)
        let subject = KaraokeContinuityLifecycle(audio: audio)
        let token = subject.beginSelection()
        XCTAssertTrue(subject.selected(token: token))
        XCTAssertEqual(audio.starts, 0)
        XCTAssertEqual(subject.state, .connectedMuted)
        subject.setGain(10)
        await subject.enable()
        XCTAssertEqual(subject.state, .monitoring)
        XCTAssertEqual(audio.gain, 0.25)
        subject.setGain(.nan)
        XCTAssertEqual(audio.gain, 0)
        subject.interrupted()
        XCTAssertFalse(audio.active)
        await subject.enable()
        XCTAssertEqual(audio.starts, 1, "Interruption must require a new selection")
    }
    @MainActor func testMutePersistsAcrossLevelChangesAndDisconnectCannotUnmute() async {
        let audio = ContinuityAudioFake(), model = KaraokeContinuityLifecycle(audio: ContinuityAudioFake())
        model.setMuted(false)
        XCTAssertTrue(model.isMuted)
        let subject = KaraokeContinuityLifecycle(audio: audio)
        XCTAssertTrue(subject.selected(token: subject.beginSelection()))
        await subject.enable()
        XCTAssertFalse(subject.isMuted)
        subject.setMuted(true); subject.setGain(0.2)
        XCTAssertEqual(audio.gain, 0)
        subject.setMuted(false)
        XCTAssertEqual(audio.gain, 0.2)
        subject.stop(); subject.setMuted(false)
        XCTAssertTrue(subject.isMuted)
        XCTAssertFalse(audio.active)
    }
    @MainActor func testDeniedPermissionNeverStarts() async {
        let audio = ContinuityAudioFake(); audio.permission = false
        let model = KaraokeContinuityLifecycle(audio: audio)
        XCTAssertTrue(model.selected(token: model.beginSelection()))
        await model.enable()
        XCTAssertEqual(model.state, .denied)
        XCTAssertEqual(audio.starts, 0)
    }
    @MainActor func testLatePickerAndPermissionAfterStopAreRejected() async {
        let audio = ContinuityAudioFake(); audio.suspend = true
        let model = KaraokeContinuityLifecycle(audio: audio)
        let token = model.beginSelection()
        XCTAssertTrue(model.selected(token: token))
        let enable = Task { await model.enable() }
        // Yield until the fake has installed its permission continuation, with a bounded loop.
        for _ in 0..<100 where audio.pending == nil { await Task.yield() }
        XCTAssertNotNil(audio.pending)
        model.stop()
        audio.pending?.resume(returning: true); audio.pending = nil
        await enable.value
        XCTAssertEqual(model.state, .off)
        XCTAssertEqual(audio.starts, 0)
        XCTAssertFalse(model.selected(token: token))
    }
    @MainActor func testTaskCancellationAndDisconnectDuringPermission() async {
        for disconnect in [false, true] {
            let audio = ContinuityAudioFake(); audio.suspend = true
            let model = KaraokeContinuityLifecycle(audio: audio)
            XCTAssertTrue(model.selected(token: model.beginSelection()))
            let enable = Task { await model.enable() }
            for _ in 0..<100 where audio.pending == nil { await Task.yield() }
            XCTAssertNotNil(audio.pending)
            if disconnect { model.interrupted() } else { enable.cancel() }
            audio.pending?.resume(returning: true); audio.pending = nil
            await enable.value
            XCTAssertEqual(audio.starts, 0)
            XCTAssertFalse(audio.active)
            XCTAssertEqual(model.state, disconnect ? .interrupted : .off)
        }
    }
    @MainActor func testPartialStartFailureRestoresAndReportsRestorationFailure() async {
        let audio = ContinuityAudioFake(); audio.failStart = true
        let model = KaraokeContinuityLifecycle(audio: audio)
        XCTAssertTrue(model.selected(token: model.beginSelection()))
        await model.enable()
        XCTAssertEqual(model.state, .failed)
        XCTAssertFalse(audio.active)
        XCTAssertEqual(audio.stops, 2)
        audio.failRestore = true
        model.stop()
        XCTAssertTrue(model.status.contains("restoration failed"))
        let token = model.beginSelection()
        XCTAssertEqual(model.state, .failed)
        XCTAssertFalse(model.selected(token: token))
    }
    @MainActor func testCancelSelectionDoesNotCancelConnectedDeviceOrNewSelection() {
        let model = KaraokeContinuityLifecycle(audio: ContinuityAudioFake())
        let old = model.beginSelection(), current = model.beginSelection()
        model.cancelSelection(token: old)
        XCTAssertEqual(model.state, .selecting)
        XCTAssertTrue(model.selected(token: current))
        model.cancelSelection(token: current)
        XCTAssertEqual(model.state, .connectedMuted)
        model.stop(); model.stop()
        XCTAssertEqual(model.state, .off)
    }
}
