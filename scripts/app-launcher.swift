// ARCHiVE.app launcher
//
// Double-click → runs the bundled `archive ui` server (which auto-opens the
// browser on a loopback bind). Quitting the app stops the server.
// Drag-and-drop of a backup folder onto the app icon prefills `--backup`.
import Foundation

let resources = Bundle.main.resourceURL!

func findArchive() -> URL {
    // Dev layout (running from the build dir) and packaged layout.
    for base in [resources, Bundle.main.executableURL!.deletingLastPathComponent()] {
        let candidate = base.appendingPathComponent("archive")
        if FileManager.default.fileExists(atPath: candidate.path) { return candidate }
    }
    // Bundled Resources/archive is the normal case.
    return resources.appendingPathComponent("archive")
}

let archive = findArchive()

var args = ["ui"]
// Drag-and-drop: first argument that is an existing directory prefills --backup.
for raw in ProcessInfo.processInfo.arguments.dropFirst() {
    var isDir: ObjCBool = false
    if FileManager.default.fileExists(atPath: raw, isDirectory: &isDir), isDir.boolValue {
        args = ["ui", "--backup", raw]
        break
    }
}

let child = Process()
child.executableURL = archive
child.arguments = args
// The child inherits our stdout/stderr (visible in Console.app as ARCHiVE).

// The server is gone — exit with it, but surface a non-zero exit to the user
// (a double-launch with port 8099 busy is the realistic failure: the second
// app must not sit silently in the Dock).
final class QuitFlag: @unchecked Sendable {
    var value = false
}
let quitting = QuitFlag() // SIGTERM-initiated quit: no error dialog
func errorDialog(_ text: String) {
    // AppleScript strings need \\ escaped, too; newlines are not allowed.
    var safe = text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "'")
    safe = safe.replacingOccurrences(of: "\n", with: " ")
    let alert = "display dialog \"" + safe + "\" buttons [\"OK\"] with icon stop"
    _ = try? Process.run(URL(fileURLWithPath: "/usr/bin/osascript"), arguments: ["-e", alert])
}
child.terminationHandler = { proc in
    if proc.terminationStatus != 0 && !quitting.value {
        errorDialog("ARCHiVE server se ukončil s chybou (kód \(proc.terminationStatus)). Detaily najdeš v Console.app pod ARCHiVE.")
    }
    exit(proc.terminationStatus)
}

do {
    try child.run()
} catch {
    errorDialog("ARCHiVE nelze spustit: \(error.localizedDescription)")
    exit(1)
}
// A child that dies within the first microseconds is handled here.
if !child.isRunning { exit(child.terminationStatus) }

// Quitting the app must stop the server: SIGTERM/SIGINT/SIGHUP to us kills
// the child. Handlers live on the main queue, and the main thread must run
// the loop (a blocking waitUntilExit would starve them — the signal sources
// would never fire and the process would ignore the signals).
let killChild: @Sendable (Int32) -> Void = { sig in
    quitting.value = true
    if child.isRunning {
        child.terminate()
        // Grace, then escalate: a child that ignores/z stalls on SIGTERM must
        // not hang the Quit path (the server would stay bound to port 8099).
        let deadline = Date().addingTimeInterval(3)
        while child.isRunning && Date() < deadline {
            usleep(100_000)
        }
        if child.isRunning {
            kill(child.processIdentifier, SIGKILL)
        }
        child.waitUntilExit() // don't exit before the server is down
    }
    exit(sig)
}
// The dispatch sources must be retained for as long as they are needed —
// a deallocated source is cancelled and the signal is then simply ignored.
var signalSources: [DispatchSourceSignal] = []
for sig in [SIGTERM, SIGINT, SIGHUP] {
    signal(sig, SIG_IGN)
    let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
    src.setEventHandler(handler: { killChild(sig) })
    src.resume()
    signalSources.append(src)
}

// Single exit path: the server is gone (user quit it, or it crashed/failed to
// bind) — exit with it, and surface a non-zero exit to the user (a
// double-launch with port 8099 busy is the realistic failure: the second app
// must not sit silently in the Dock).
RunLoop.main.run() // never returns; dispatches the signal sources above
