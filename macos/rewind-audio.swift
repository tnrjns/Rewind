// rewind-audio: streams the Mac's system audio to stdout as 48 kHz stereo s16le PCM.
// Uses ScreenCaptureKit (macOS 13+). Exits when stdin closes.
import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

let rate = 48_000
let channels = 2
let out = FileHandle.standardOutput

final class Sink: NSObject, SCStreamOutput, SCStreamDelegate {
    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, sb.isValid else { return }
        var needed = 0
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: &needed, bufferListOut: nil, bufferListSize: 0,
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil, flags: 0, blockBufferOut: nil)
        guard needed > 0 else { return }
        let raw = UnsafeMutableRawPointer.allocate(byteCount: needed, alignment: 16)
        defer { raw.deallocate() }
        let list = raw.bindMemory(to: AudioBufferList.self, capacity: 1)
        var block: CMBlockBuffer?
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: nil, bufferListOut: list, bufferListSize: needed,
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, blockBufferOut: &block)
        guard status == noErr else { return }
        let frames = CMSampleBufferGetNumSamples(sb)
        let buffers = UnsafeMutableAudioBufferListPointer(list)
        var pcm = [Int16](repeating: 0, count: frames * channels)
        if buffers.count >= channels {
            for c in 0..<channels {
                guard let p = buffers[c].mData?.assumingMemoryBound(to: Float.self) else { continue }
                for i in 0..<frames { pcm[i * channels + c] = Int16(max(-1, min(1, p[i])) * 32767) }
            }
        } else if let p = buffers[0].mData?.assumingMemoryBound(to: Float.self) {
            let n = Int(buffers[0].mDataByteSize) / MemoryLayout<Float>.size
            for i in 0..<min(n, frames * channels) { pcm[i] = Int16(max(-1, min(1, p[i])) * 32767) }
        }
        pcm.withUnsafeBytes { out.write(Data($0)) }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileHandle.standardError.write("stopped: \(error.localizedDescription)\n".data(using: .utf8)!)
        exit(2)
    }
}

let sink = Sink()
var keep: SCStream?

Task {
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        guard let display = content.displays.first else { throw NSError(domain: "rewind", code: 1) }
        let filter = SCContentFilter(display: display, excludingWindows: [])
        let cfg = SCStreamConfiguration()
        cfg.capturesAudio = true
        cfg.excludesCurrentProcessAudio = true
        cfg.sampleRate = rate
        cfg.channelCount = channels
        cfg.width = 2
        cfg.height = 2
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        let stream = SCStream(filter: filter, configuration: cfg, delegate: sink)
        try stream.addStreamOutput(sink, type: .audio, sampleHandlerQueue: DispatchQueue(label: "audio"))
        // ScreenCaptureKit only delivers audio while a screen output exists; its 2x2 frames are ignored
        try stream.addStreamOutput(sink, type: .screen, sampleHandlerQueue: DispatchQueue(label: "screen"))
        try await stream.startCapture()
        keep = stream
        FileHandle.standardError.write("ready\n".data(using: .utf8)!)
    } catch {
        FileHandle.standardError.write("error: \(error.localizedDescription)\n".data(using: .utf8)!)
        exit(1)
    }
}

DispatchQueue.global().async {
    _ = FileHandle.standardInput.readDataToEndOfFile()
    exit(0)
}
dispatchMain()
