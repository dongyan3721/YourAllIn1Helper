// Native macOS Cocoa + WKWebView + Carbon HotKey + CoreGraphics/AXUIElement Host
// (`src-tauri/macos/NativeHost.swift`)
//
// Provides real macOS desktop capabilities for All-in-One Toolbox:
// 1. Real system tray icon (`NSStatusItem`) & native menu.
// 2. Real system-wide global shortcuts (`Carbon RegisterEventHotKey`):
//    - Cmd+Shift+V: Clipboard Popup at mouse cursor
//    - Cmd+Shift+N: New Pinned Sticky Note
//    - Cmd+Shift+P: Toggle Always-on-Top on active external window
//    - Cmd+Option+P: Fullscreen crosshair picker to pin any real macOS window
// 3. Real `NSPasteboard.general.changeCount` 250ms clipboard listener with source app tracking.
// 4. Real on-demand `NSWindow` + `WKWebView` pool (`WindowPoolManager` Level 0/1/2 lifecycle)
//    with true `NSWindow.Level.floating` always-on-top and `alphaValue` opacity.
// 5. Real external window enumeration (`CGWindowListCopyWindowInfo`), floating `.floating`-level
//    live window projection when covered by other apps, red DeskPin control bar (`📌`),
//    and AXUIElement / System Events window activation.

import Cocoa
import WebKit
import Carbon
import ApplicationServices

// MARK: - IPC Client Bridge to Rust Microkernel (`http://127.0.0.1:14200/api/ipc`)

final class RustIpcBridge {
    static let shared = RustIpcBridge()
    let port: Int = 14200
    var baseURL: String { "http://127.0.0.1:\(port)" }

    func invoke(_ cmd: String, args: [String: Any] = [:], completion: (([String: Any]?) -> Void)? = nil) {
        guard let url = URL(string: "\(baseURL)/api/ipc") else {
            completion?(nil)
            return
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.timeoutInterval = 2.0
        let body: [String: Any] = ["cmd": cmd, "args": args]
        req.httpBody = try? JSONSerialization.data(withJSONObject: body, options: [])

        URLSession.shared.dataTask(with: req) { data, _, _ in
            guard let data = data,
                  let json = try? JSONSerialization.jsonObject(with: data, options: []) as? [String: Any],
                  let ok = json["ok"] as? Bool, ok else {
                DispatchQueue.main.async { completion?(nil) }
                return
            }
            DispatchQueue.main.async {
                if let dict = json["data"] as? [String: Any] {
                    completion?(dict)
                } else if let arr = json["data"] as? [[String: Any]] {
                    completion?(["items": arr])
                } else {
                    completion?([:])
                }
            }
        }.resume()
    }
}

// MARK: - Custom Borderless / Floating NSWindow that can become Key Window for typing

final class ManagedWebViewWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

// MARK: - Clickable Mirror View (clicking the floating pinned window projection activates the real window)

final class ClickableMirrorImageView: NSImageView {
    var onClicked: (() -> Void)?

    override func mouseDown(with event: NSEvent) {
        onClicked?()
    }
}

// MARK: - Fullscreen Crosshair Picker Overlay (`window-pin-picker`)

final class CrosshairPickerView: NSView {
    var candidateWindows: [[String: Any]] = []
    var hoveredWindow: [String: Any]?
    var onWindowPicked: ((Int, Int) -> Void)?
    var onCancel: (() -> Void)?
    private var trackingArea: NSTrackingArea?

    override var acceptsFirstResponder: Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .activeAlways, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .crosshair)
    }

    override func mouseMoved(with event: NSEvent) {
        guard let screen = NSScreen.main else { return }
        let loc = NSEvent.mouseLocation
        let qx = Int(loc.x)
        let qy = Int(screen.frame.height - loc.y)

        hoveredWindow = candidateWindows.first { win in
            let x = win["x"] as? Int ?? 0
            let y = win["y"] as? Int ?? 0
            let w = win["width"] as? Int ?? 0
            let h = win["height"] as? Int ?? 0
            return qx >= x && qx < (x + w) && qy >= y && qy < (y + h)
        }
        needsDisplay = true
    }

    override func mouseDown(with event: NSEvent) {
        guard let screen = NSScreen.main else { return }
        let loc = NSEvent.mouseLocation
        let qx = Int(loc.x)
        let qy = Int(screen.frame.height - loc.y)
        onWindowPicked?(qx, qy)
    }

    override func rightMouseDown(with event: NSEvent) {
        onCancel?()
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { // Escape
            onCancel?()
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedWhite: 0.0, alpha: 0.22).setFill()
        dirtyRect.fill()

        guard let screen = NSScreen.main else { return }
        let screenH = screen.frame.height

        let bannerRect = NSRect(x: (bounds.width - 560) / 2, y: bounds.height - 70, width: 560, height: 42)
        let bannerPath = NSBezierPath(roundedRect: bannerRect, xRadius: 21, yRadius: 21)
        NSColor(calibratedRed: 0.06, green: 0.09, blue: 0.16, alpha: 0.92).setFill()
        bannerPath.fill()
        NSColor(calibratedRed: 0.22, green: 0.74, blue: 0.97, alpha: 0.9).setStroke()
        bannerPath.lineWidth = 1.5
        bannerPath.stroke()

        let bannerText = "🎯 移动鼠标选择任意桌面窗口并左键点击将其【置顶最前】（右键或按 ESC 取消）"
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.boldSystemFont(ofSize: 13),
            .foregroundColor: NSColor.white
        ]
        let size = bannerText.size(withAttributes: attrs)
        bannerText.draw(
            at: NSPoint(x: bannerRect.midX - size.width / 2, y: bannerRect.midY - size.height / 2),
            withAttributes: attrs
        )

        if let win = hoveredWindow {
            let qx = CGFloat(win["x"] as? Int ?? 0)
            let qy = CGFloat(win["y"] as? Int ?? 0)
            let qw = CGFloat(win["width"] as? Int ?? 0)
            let qh = CGFloat(win["height"] as? Int ?? 0)
            let cocoaY = screenH - qy - qh
            let highlightRect = NSRect(x: qx, y: cocoaY, width: qw, height: qh)

            NSColor(calibratedRed: 0.96, green: 0.25, blue: 0.37, alpha: 0.18).setFill()
            highlightRect.fill()

            let path = NSBezierPath(rect: highlightRect.insetBy(dx: 1.5, dy: 1.5))
            NSColor(calibratedRed: 0.96, green: 0.25, blue: 0.37, alpha: 0.95).setStroke()
            path.lineWidth = 3.0
            path.stroke()

            let proc = win["process_name"] as? String ?? "App"
            let title = win["title"] as? String ?? ""
            let labelStr = "📌 点击置顶最前: \(proc) — \(title)"
            let labelSize = labelStr.size(withAttributes: attrs)
            let tagRect = NSRect(
                x: highlightRect.minX + 10,
                y: min(bounds.height - 90, highlightRect.maxY - 34),
                width: labelSize.width + 20,
                height: 26
            )
            let tagPath = NSBezierPath(roundedRect: tagRect, xRadius: 6, yRadius: 6)
            NSColor(calibratedRed: 0.88, green: 0.11, blue: 0.28, alpha: 0.95).setFill()
            tagPath.fill()
            labelStr.draw(at: NSPoint(x: tagRect.minX + 10, y: tagRect.minY + 5), withAttributes: attrs)
        }
    }
}

// MARK: - Real Topmost External Window Pin Controller (Badge + Live `.floating` Window Projection)

final class DeskPinTopmostController: NSObject {
    let handle: UInt64
    let badgeWindow: NSPanel
    let mirrorPanel: NSPanel
    private let titleField = NSTextField(labelWithString: "")
    private let mirrorImageView = ClickableMirrorImageView(frame: .zero)
    private var captureTimer: Timer?
    private let tempImgPath: String
    private var isCapturing: Bool = false

    var onActivateTarget: ((UInt64) -> Void)?
    var onUnpinTarget: ((UInt64) -> Void)?

    init(handle: UInt64, info: [String: Any]) {
        self.handle = handle
        self.tempImgPath = NSTemporaryDirectory() + "aio_pin_\(handle).jpg"

        // 1. Floating `.floating`-level live window mirror panel (keeps external window visible
        //    on top of all other apps even when another app has keyboard focus!)
        let mPanel = NSPanel(
            contentRect: NSRect(x: 100, y: 100, width: 400, height: 300),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        mPanel.level = .floating
        mPanel.isOpaque = false
        mPanel.backgroundColor = .clear
        mPanel.hasShadow = true
        mPanel.hidesOnDeactivate = false
        mPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        self.mirrorPanel = mPanel

        // 2. Floating red DeskPin control badge at top-right of the pinned window
        let bPanel = NSPanel(
            contentRect: NSRect(x: 200, y: 200, width: 300, height: 30),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        bPanel.level = .statusBar
        bPanel.isOpaque = false
        bPanel.backgroundColor = .clear
        bPanel.hasShadow = true
        bPanel.hidesOnDeactivate = false
        bPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        self.badgeWindow = bPanel

        super.init()

        mirrorImageView.imageScaling = .scaleAxesIndependently
        mirrorImageView.wantsLayer = true
        mirrorImageView.layer?.cornerRadius = 8
        mirrorImageView.layer?.masksToBounds = true
        mirrorImageView.layer?.borderWidth = 2.0
        mirrorImageView.layer?.borderColor = NSColor(calibratedRed: 0.96, green: 0.25, blue: 0.37, alpha: 0.85).cgColor
        mirrorImageView.onClicked = { [weak self] in
            guard let self = self else { return }
            self.onActivateTarget?(self.handle)
        }
        mPanel.contentView = mirrorImageView

        let container = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 30))
        container.wantsLayer = true
        container.layer?.backgroundColor = NSColor(calibratedRed: 0.88, green: 0.11, blue: 0.28, alpha: 0.95).cgColor
        container.layer?.cornerRadius = 15
        container.layer?.borderWidth = 1.0
        container.layer?.borderColor = NSColor.white.withAlphaComponent(0.35).cgColor

        titleField.font = NSFont.boldSystemFont(ofSize: 11)
        titleField.textColor = .white
        titleField.lineBreakMode = .byTruncatingTail
        titleField.frame = NSRect(x: 12, y: 6, width: 176, height: 18)
        container.addSubview(titleField)

        let focusBtn = NSButton(frame: NSRect(x: 190, y: 3, width: 50, height: 24))
        focusBtn.title = "激活"
        focusBtn.bezelStyle = .roundRect
        focusBtn.font = NSFont.boldSystemFont(ofSize: 10)
        focusBtn.target = self
        focusBtn.action = #selector(handleFocusClicked)
        container.addSubview(focusBtn)

        let unpinBtn = NSButton(frame: NSRect(x: 244, y: 3, width: 50, height: 24))
        unpinBtn.title = "取消"
        unpinBtn.bezelStyle = .roundRect
        unpinBtn.font = NSFont.boldSystemFont(ofSize: 10)
        unpinBtn.target = self
        unpinBtn.action = #selector(handleUnpinClicked)
        container.addSubview(unpinBtn)

        bPanel.contentView = container
        updateState(with: info, isTargetAppFrontmost: true)
        bPanel.orderFrontRegardless()

        // Periodically capture the pinned window's backing store so when another app is active,
        // the `.floating` panel keeps the pinned window visually on top of all windows!
        captureSnapshotAsync()
        captureTimer = Timer.scheduledTimer(
            timeInterval: 0.45,
            target: self,
            selector: #selector(onCaptureTick),
            userInfo: nil,
            repeats: true
        )
    }

    @objc private func onCaptureTick() {
        captureSnapshotAsync()
    }

    private func captureSnapshotAsync() {
        if isCapturing { return }
        isCapturing = true
        let winId = self.handle
        let outPath = self.tempImgPath

        DispatchQueue.global(qos: .userInteractive).async { [weak self] in
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            proc.arguments = ["-l", "\(winId)", "-x", "-o", "-t", "jpg", outPath]
            try? proc.run()
            proc.waitUntilExit()

            let img = NSImage(contentsOfFile: outPath)
            DispatchQueue.main.async {
                self?.isCapturing = false
                if let img = img {
                    self?.mirrorImageView.image = img
                }
            }
        }
    }

    func updateState(with info: [String: Any], isTargetAppFrontmost: Bool) {
        guard let screen = NSScreen.main else { return }
        let proc = info["process_name"] as? String ?? "Window"
        let title = info["title"] as? String ?? ""
        titleField.stringValue = "📌 已置顶: \(proc) — \(title)"

        let qx = CGFloat(info["x"] as? Int ?? 100)
        let qy = CGFloat(info["y"] as? Int ?? 100)
        let qw = CGFloat(info["width"] as? Int ?? 600)
        let qh = CGFloat(info["height"] as? Int ?? 400)
        let screenH = screen.frame.height
        let cocoaY = screenH - qy - qh

        let winFrame = NSRect(x: qx, y: cocoaY, width: qw, height: qh)
        mirrorPanel.setFrame(winFrame, display: true)
        mirrorImageView.frame = NSRect(origin: .zero, size: winFrame.size)

        // When the user is directly interacting with the pinned app itself, hide the mirror overlay
        // so the user types/clicks directly in the real window; when the user switches to ANY other app,
        // show the `.floating` topmost projection so the pinned window stays on top of everything!
        if isTargetAppFrontmost {
            mirrorPanel.orderOut(nil)
        } else if mirrorImageView.image != nil {
            mirrorPanel.orderFrontRegardless()
        }

        let badgeW: CGFloat = 300
        let badgeH: CGFloat = 30
        let bx = max(10, min(screen.frame.width - badgeW - 10, qx + qw - badgeW - 12))
        let by = max(40, min(screenH - badgeH - 30, screenH - qy - badgeH - 6))
        badgeWindow.setFrameOrigin(NSPoint(x: bx, y: by))
        badgeWindow.orderFrontRegardless()
    }

    @objc private func handleFocusClicked() {
        onActivateTarget?(handle)
    }

    @objc private func handleUnpinClicked() {
        onUnpinTarget?(handle)
    }

    func close() {
        captureTimer?.invalidate()
        captureTimer = nil
        mirrorPanel.close()
        badgeWindow.close()
        try? FileManager.default.removeItem(atPath: tempImgPath)
    }
}

// MARK: - Main Application Delegate & WindowPool Synchronizer

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var statusItem: NSStatusItem!
    private var daemonProcess: Process?
    private var syncTimer: Timer?
    private var clipboardTimer: Timer?

    private var lastChangeCount: Int = NSPasteboard.general.changeCount
    private var lastPasteTriggerCount: Int = 0
    private var lastExternalApp: NSRunningApplication?

    // Active native windows mapped by `WindowPoolManager` label (`dashboard`, `clipboard-popup`, `sticky-note-{id}`, `window-pin-picker`)
    private var nativeWindows: [String: NSWindow] = [:]
    private var nativeWebViews: [String: WKWebView] = [:]

    // Active DeskPin topmost controllers mapped by external `CGWindowID`
    private var deskPinControllers: [UInt64: DeskPinTopmostController] = [:]
    private var latestOsWindows: [[String: Any]] = []

    // Carbon HotKey references
    private var hotKeyRefs: [EventHotKeyRef?] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        setupMainMenu()
        setupStatusBar()
        ensureRustDaemonRunning()
        registerGlobalCarbonHotKeys()

        NSWorkspace.shared.notificationCenter.addObserver(
            self,
            selector: #selector(onActiveAppChanged(_:)),
            name: NSWorkspace.didActivateApplicationNotification,
            object: nil
        )

        clipboardTimer = Timer.scheduledTimer(
            timeInterval: 0.25,
            target: self,
            selector: #selector(pollSystemClipboard),
            userInfo: nil,
            repeats: true
        )

        syncTimer = Timer.scheduledTimer(
            timeInterval: 0.25,
            target: self,
            selector: #selector(syncWithRustKernel),
            userInfo: nil,
            repeats: true
        )

        let args = CommandLine.arguments
        if !args.contains("--silent") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) {
                RustIpcBridge.shared.invoke("kernel_open_dashboard", args: ["route": "/overview"]) { _ in
                    self.syncWithRustKernel()
                }
            }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        let sem = DispatchSemaphore(value: 0)
        RustIpcBridge.shared.invoke("kernel_shutdown") { _ in
            sem.signal()
        }
        _ = sem.wait(timeout: .now() + 1.0)

        for ctrl in deskPinControllers.values {
            ctrl.close()
        }
        deskPinControllers.removeAll()

        if let proc = daemonProcess, proc.isRunning {
            proc.terminate()
        }
    }

    // MARK: - Daemon Process Management

    private func ensureRustDaemonRunning() {
        let execURL = Bundle.main.executableURL
        let siblingDaemon = execURL?.deletingLastPathComponent().appendingPathComponent("all-in-one-desktop")

        var candidates: [String] = []
        if let p = siblingDaemon?.path {
            candidates.append(p)
        }
        candidates.append(FileManager.default.currentDirectoryPath + "/target/release/all-in-one-desktop")
        candidates.append(FileManager.default.currentDirectoryPath + "/target/debug/all-in-one-desktop")

        guard let daemonPath = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0) }) else {
            return
        }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: daemonPath)
        proc.arguments = ["--no-open"]
        try? proc.run()
        self.daemonProcess = proc
    }

    // MARK: - Menu Bar Tray (`NSStatusItem`) & Main Menu

    private func setupMainMenu() {
        let mainMenu = NSMenu()
        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "打开主控台 (Dashboard)", action: #selector(openDashboardAction), keyEquivalent: "d")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "退出 All-in-One Toolbox", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu
        NSApp.mainMenu = mainMenu
    }

    private func setupStatusBar() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "🛠️ 0 WV"

        let menu = NSMenu()
        menu.addItem(withTitle: "🎛️ 打开主控台 (Dashboard)", action: #selector(openDashboardAction), keyEquivalent: "")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(withTitle: "📋 剪贴板快贴弹窗 (⌘+⇧+V)", action: #selector(toggleClipboardPopupAction), keyEquivalent: "")
        menu.addItem(withTitle: "📝 新建桌面置顶便笺 (⌘+⇧+N)", action: #selector(createStickyNoteAction), keyEquivalent: "")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(withTitle: "📌 置顶/取消前序活动窗口 (⌘+⇧+P)", action: #selector(toggleForegroundWindowPinAction), keyEquivalent: "")
        menu.addItem(withTitle: "🎯 准星拾取任意窗口置顶 (⌘+⌥+P)", action: #selector(startCrosshairPickerAction), keyEquivalent: "")
        menu.addItem(withTitle: "🔄 还原所有已置顶外部窗口", action: #selector(restoreAllPinnedWindowsAction), keyEquivalent: "")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(withTitle: "退出 All-in-One Toolbox", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        statusItem.menu = menu
    }

    @objc private func onActiveAppChanged(_ note: Notification) {
        guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
        if app.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            lastExternalApp = app
        }
    }

    // MARK: - Menu & Global HotKey Actions

    @objc func openDashboardAction() {
        RustIpcBridge.shared.invoke("kernel_open_dashboard", args: ["route": "/overview"]) { _ in
            self.syncWithRustKernel()
        }
    }

    @objc func toggleClipboardPopupAction() {
        RustIpcBridge.shared.invoke("kernel_trigger_shortcut", args: ["action_id": "clipboard:toggle_popup"]) { _ in
            self.syncWithRustKernel()
        }
    }

    @objc func createStickyNoteAction() {
        RustIpcBridge.shared.invoke("kernel_trigger_shortcut", args: ["action_id": "sticky_note:create"]) { _ in
            self.syncWithRustKernel()
        }
    }

    @objc func toggleForegroundWindowPinAction() {
        syncRealMacOSWindowsToRust {
            RustIpcBridge.shared.invoke("winpin_toggle_foreground") { resp in
                if let win = resp?["window"] as? [String: Any],
                   let isPinned = resp?["is_pinned"] as? Bool,
                   let handle = win["handle"] as? UInt64, isPinned {
                    self.raiseExternalMacOSWindow(handle: handle, info: win)
                }
                self.syncWithRustKernel()
            }
        }
    }

    @objc func startCrosshairPickerAction() {
        syncRealMacOSWindowsToRust {
            RustIpcBridge.shared.invoke("kernel_trigger_shortcut", args: ["action_id": "window_pin:pick_cursor"]) { _ in
                self.syncWithRustKernel()
            }
        }
    }

    @objc func restoreAllPinnedWindowsAction() {
        RustIpcBridge.shared.invoke("winpin_restore_all") { _ in
            self.syncWithRustKernel()
        }
    }

    // MARK: - Carbon System-Wide Global HotKeys (`⌘+⇧+V`, `⌘+⇧+N`, `⌘+⇧+P`, `⌘+⌥+P`)

    private func registerGlobalCarbonHotKeys() {
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )

        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(
            GetApplicationEventTarget(),
            { (_, event, userData) -> OSStatus in
                guard let event = event, let userData = userData else { return noErr }
                var hotKeyID = EventHotKeyID()
                GetEventParameter(
                    event,
                    EventParamName(kEventParamDirectObject),
                    EventParamType(typeEventHotKeyID),
                    nil,
                    MemoryLayout<EventHotKeyID>.size,
                    nil,
                    &hotKeyID
                )
                let delegate = Unmanaged<AppDelegate>.fromOpaque(userData).takeUnretainedValue()
                DispatchQueue.main.async {
                    switch hotKeyID.id {
                    case 1: delegate.toggleClipboardPopupAction()
                    case 2: delegate.createStickyNoteAction()
                    case 3: delegate.toggleForegroundWindowPinAction()
                    case 4: delegate.startCrosshairPickerAction()
                    default: break
                    }
                }
                return noErr
            },
            1,
            &eventType,
            selfPtr,
            nil
        )

        registerSingleHotKey(id: 1, keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(cmdKey | shiftKey))
        registerSingleHotKey(id: 2, keyCode: UInt32(kVK_ANSI_N), modifiers: UInt32(cmdKey | shiftKey))
        registerSingleHotKey(id: 3, keyCode: UInt32(kVK_ANSI_P), modifiers: UInt32(cmdKey | shiftKey))
        registerSingleHotKey(id: 4, keyCode: UInt32(kVK_ANSI_P), modifiers: UInt32(cmdKey | optionKey))
    }

    private func registerSingleHotKey(id: UInt32, keyCode: UInt32, modifiers: UInt32) {
        var ref: EventHotKeyRef?
        let hotKeyID = EventHotKeyID(signature: OSType(0x41494F54), id: id)
        RegisterEventHotKey(keyCode, modifiers, hotKeyID, GetApplicationEventTarget(), 0, &ref)
        hotKeyRefs.append(ref)
    }

    // MARK: - Real System Clipboard Polling (`NSPasteboard.general.changeCount`)

    @objc private func pollSystemClipboard() {
        let pb = NSPasteboard.general
        let currentCount = pb.changeCount
        guard currentCount != lastChangeCount else { return }
        lastChangeCount = currentCount

        guard let copiedText = pb.string(forType: .string),
              !copiedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }

        let sourceApp = NSWorkspace.shared.frontmostApplication?.localizedName ?? "macOS"
        RustIpcBridge.shared.invoke(
            "clipboard_capture_text",
            args: ["text": copiedText, "source_app_name": sourceApp]
        )
    }

    // MARK: - Real macOS Window Enumeration (`CGWindowListCopyWindowInfo`) & Activation

    private func enumerateRealMacOSWindows() -> ([[String: Any]], UInt64?) {
        let myPid = Int(ProcessInfo.processInfo.processIdentifier)
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        guard let infoList = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
            return ([], nil)
        }

        var results: [[String: Any]] = []
        var fgHandle: UInt64?
        let preferredExtPid = lastExternalApp.map { Int($0.processIdentifier) }

        for entry in infoList {
            let layer = entry[kCGWindowLayer as String] as? Int ?? 0
            if layer != 0 { continue }

            let ownerPid = entry[kCGWindowOwnerPID as String] as? Int ?? 0
            if ownerPid == myPid { continue }

            guard let bounds = entry[kCGWindowBounds as String] as? [String: Any] else { continue }
            let x = Int(bounds["X"] as? CGFloat ?? 0)
            let y = Int(bounds["Y"] as? CGFloat ?? 0)
            let w = Int(bounds["Width"] as? CGFloat ?? 0)
            let h = Int(bounds["Height"] as? CGFloat ?? 0)
            if w < 140 || h < 90 { continue }

            let winNum = UInt64(entry[kCGWindowNumber as String] as? Int ?? 0)
            if winNum == 0 { continue }

            let ownerName = entry[kCGWindowOwnerName as String] as? String ?? "macOS App"
            let rawTitle = entry[kCGWindowName as String] as? String ?? ""
            let displayTitle = rawTitle.isEmpty ? "\(ownerName) 窗口 (#\(winNum))" : rawTitle

            if fgHandle == nil {
                if let pref = preferredExtPid, ownerPid == pref {
                    fgHandle = winNum
                } else {
                    fgHandle = winNum
                }
            }

            results.append([
                "handle": winNum,
                "title": displayTitle,
                "process_name": ownerName,
                "process_id": ownerPid,
                "x": x,
                "y": y,
                "width": w,
                "height": h,
                "is_topmost": false
            ])
        }

        return (results, fgHandle)
    }

    private func syncRealMacOSWindowsToRust(completion: (() -> Void)? = nil) {
        let (wins, fg) = enumerateRealMacOSWindows()
        self.latestOsWindows = wins
        var args: [String: Any] = ["windows": wins]
        if let fg = fg {
            args["foreground_handle"] = fg
        }
        RustIpcBridge.shared.invoke("winpin_sync_os_windows", args: args) { _ in
            completion?()
        }
    }

    /// Raises and activates an external macOS application window using AXUIElement + NSRunningApplication.
    private func raiseExternalMacOSWindow(handle: UInt64, info: [String: Any]) {
        let pid = pid_t(info["process_id"] as? Int ?? 0)
        guard pid > 0 else { return }

        let appElem = AXUIElementCreateApplication(pid)
        var windowsRef: CFTypeRef?
        if AXUIElementCopyAttributeValue(appElem, kAXWindowsAttribute as CFString, &windowsRef) == .success,
           let axWindows = windowsRef as? [AXUIElement] {
            for axWin in axWindows {
                AXUIElementPerformAction(axWin, kAXRaiseAction as CFString)
            }
        }

        if let runningApp = NSRunningApplication(processIdentifier: pid) {
            NSApp.yieldActivation(to: runningApp)
            runningApp.activate()
        }
    }

    // MARK: - Synchronize Rust `WindowPoolManager` & `WindowPinModule` with Native Cocoa Windows

    @objc private func syncWithRustKernel() {
        let (osWins, fgHandle) = enumerateRealMacOSWindows()
        self.latestOsWindows = osWins

        var syncArgs: [String: Any] = ["windows": osWins]
        if let fg = fgHandle {
            syncArgs["foreground_handle"] = fg
        }

        RustIpcBridge.shared.invoke("winpin_sync_os_windows", args: syncArgs) { pinData in
            let pinnedList = pinData?["pinned"] as? [[String: Any]] ?? []
            self.reconcileDeskPinOverlays(pinnedList: pinnedList)
        }

        RustIpcBridge.shared.invoke("kernel_get_status") { status in
            guard let status = status else { return }
            let count = status["active_window_count"] as? Int ?? 0
            let pinnedCount = self.deskPinControllers.count
            self.statusItem.button?.title = pinnedCount > 0
                ? "🛠️ \(count)WV · 📌\(pinnedCount)"
                : "🛠️ \(count) WV"

            let pasteCount = status["paste_trigger_count"] as? Int ?? 0
            if pasteCount > self.lastPasteTriggerCount {
                self.lastPasteTriggerCount = pasteCount
                self.simulatePasteToPreviousExternalApp()
            }

            let activeEntries = status["active_windows"] as? [[String: Any]] ?? []
            self.reconcileNativeWebViewWindows(activeEntries: activeEntries)
        }
    }

    private func reconcileDeskPinOverlays(pinnedList: [[String: Any]]) {
        var activeHandles = Set<UInt64>()
        let frontPid = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0

        for entry in pinnedList {
            guard let info = entry["info"] as? [String: Any],
                  let handle = info["handle"] as? UInt64 else { continue }
            activeHandles.insert(handle)

            let liveInfo = latestOsWindows.first(where: { ($0["handle"] as? UInt64) == handle }) ?? info
            let targetPid = pid_t(liveInfo["process_id"] as? Int ?? 0)
            let isTargetFrontmost = (frontPid == targetPid)

            if let existingCtrl = deskPinControllers[handle] {
                existingCtrl.updateState(with: liveInfo, isTargetAppFrontmost: isTargetFrontmost)
            } else {
                let ctrl = DeskPinTopmostController(handle: handle, info: liveInfo)
                ctrl.onActivateTarget = { [weak self] h in
                    guard let self = self else { return }
                    let targetInfo = self.latestOsWindows.first(where: { ($0["handle"] as? UInt64) == h }) ?? info
                    self.raiseExternalMacOSWindow(handle: h, info: targetInfo)
                }
                ctrl.onUnpinTarget = { [weak self] h in
                    RustIpcBridge.shared.invoke("winpin_unpin", args: ["handle": h]) { _ in
                        self?.syncWithRustKernel()
                    }
                }
                deskPinControllers[handle] = ctrl
                raiseExternalMacOSWindow(handle: handle, info: liveInfo)
            }
        }

        for (handle, ctrl) in deskPinControllers where !activeHandles.contains(handle) {
            ctrl.close()
            deskPinControllers.removeValue(forKey: handle)
        }
    }

    private func reconcileNativeWebViewWindows(activeEntries: [[String: Any]]) {
        var desiredLabels = Set<String>()

        for entry in activeEntries {
            guard let spec = entry["spec"] as? [String: Any],
                  let label = spec["label"] as? String else { continue }
            desiredLabels.insert(label)

            let alwaysOnTop = entry["always_on_top"] as? Bool ?? false

            if let existingWin = nativeWindows[label] {
                let targetLevel: NSWindow.Level = alwaysOnTop ? .floating : .normal
                if label != "window-pin-picker" && label != "clipboard-popup" && existingWin.level != targetLevel {
                    existingWin.level = targetLevel
                }
                if label.hasPrefix("sticky-note-") {
                    let noteId = String(label.dropFirst("sticky-note-".count))
                    RustIpcBridge.shared.invoke("sticky_get_note", args: ["id": noteId]) { note in
                        if let opacity = note?["opacity"] as? Double {
                            existingWin.alphaValue = CGFloat(opacity)
                        }
                    }
                }
                continue
            }

            spawnNativeWindow(label: label, spec: spec, alwaysOnTop: alwaysOnTop)
        }

        for (label, win) in nativeWindows where !desiredLabels.contains(label) {
            if let wv = nativeWebViews.removeValue(forKey: label) {
                wv.loadHTMLString("", baseURL: nil)
                wv.removeFromSuperview()
            }
            win.delegate = nil
            win.close()
            nativeWindows.removeValue(forKey: label)
        }
    }

    private func spawnNativeWindow(label: String, spec: [String: Any], alwaysOnTop: Bool) {
        guard let screen = NSScreen.main else { return }

        if label == "window-pin-picker" {
            let pickerWin = ManagedWebViewWindow(
                contentRect: screen.frame,
                styleMask: [.borderless],
                backing: .buffered,
                defer: false
            )
            pickerWin.level = .statusBar
            pickerWin.isOpaque = false
            pickerWin.backgroundColor = .clear
            pickerWin.hasShadow = false
            pickerWin.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]

            let pickerView = CrosshairPickerView(frame: screen.frame)
            pickerView.candidateWindows = latestOsWindows
            pickerView.onWindowPicked = { [weak self] qx, qy in
                RustIpcBridge.shared.invoke("winpin_pick_at_cursor", args: ["x": qx, "y": qy]) { resp in
                    if let win = resp?["window"] as? [String: Any],
                       let isPinned = resp?["is_pinned"] as? Bool,
                       let handle = win["handle"] as? UInt64, isPinned {
                        self?.raiseExternalMacOSWindow(handle: handle, info: win)
                    }
                    self?.syncWithRustKernel()
                }
            }
            pickerView.onCancel = { [weak self] in
                RustIpcBridge.shared.invoke("kernel_blur_window", args: ["label": "window-pin-picker"]) { _ in
                    self?.syncWithRustKernel()
                }
            }
            pickerWin.contentView = pickerView
            nativeWindows[label] = pickerWin
            NSApp.activate()
            pickerWin.makeKeyAndOrderFront(nil)
            return
        }

        let width = CGFloat(spec["width"] as? Double ?? 800.0)
        let height = CGFloat(spec["height"] as? Double ?? 600.0)
        let title = spec["title"] as? String ?? "All-in-One"

        let frame: NSRect
        if label == "clipboard-popup" {
            let mouse = NSEvent.mouseLocation
            let x = max(20, min(screen.frame.width - width - 20, mouse.x - width / 2))
            let y = max(40, min(screen.frame.height - height - 40, mouse.y - height + 40))
            frame = NSRect(x: x, y: y, width: width, height: height)
        } else if let sx = spec["x"] as? Double, let sy = spec["y"] as? Double {
            let cocoaY = max(60, screen.frame.height - CGFloat(sy) - height)
            frame = NSRect(x: CGFloat(sx), y: cocoaY, width: width, height: height)
        } else {
            let x = (screen.frame.width - width) / 2
            let y = (screen.frame.height - height) / 2
            frame = NSRect(x: x, y: y, width: width, height: height)
        }

        let style: NSWindow.StyleMask = (label == "dashboard")
            ? [.titled, .closable, .miniaturizable, .resizable]
            : [.titled, .closable, .resizable, .fullSizeContentView]

        let win = ManagedWebViewWindow(
            contentRect: frame,
            styleMask: style,
            backing: .buffered,
            defer: false
        )
        win.title = title
        win.isReleasedWhenClosed = false
        win.delegate = self

        if label != "dashboard" {
            win.titleVisibility = .hidden
            win.titlebarAppearsTransparent = true
            win.isMovableByWindowBackground = true
        }

        if label == "clipboard-popup" {
            win.level = .popUpMenu
            win.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        } else if alwaysOnTop {
            win.level = .floating
            win.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        } else {
            win.level = .normal
        }

        let config = WKWebViewConfiguration()
        let webView = WKWebView(frame: NSRect(origin: .zero, size: frame.size), configuration: config)
        webView.autoresizingMask = [.width, .height]

        let urlString: String
        if label == "dashboard" {
            urlString = "\(RustIpcBridge.shared.baseURL)/?mode=native&window=dashboard"
        } else if label == "clipboard-popup" {
            urlString = "\(RustIpcBridge.shared.baseURL)/?mode=native&window=clipboard-popup"
        } else if label.hasPrefix("sticky-note-") {
            let noteId = String(label.dropFirst("sticky-note-".count))
            urlString = "\(RustIpcBridge.shared.baseURL)/?mode=native&window=sticky-note&id=\(noteId)"
        } else {
            urlString = RustIpcBridge.shared.baseURL
        }

        if let url = URL(string: urlString) {
            webView.load(URLRequest(url: url))
        }

        win.contentView = webView
        nativeWindows[label] = win
        nativeWebViews[label] = webView

        NSApp.activate()
        win.makeKeyAndOrderFront(nil)
    }

    // MARK: - NSWindowDelegate (Close & Blur Lifecycle Hooks)

    func windowWillClose(_ notification: Notification) {
        guard let closingWin = notification.object as? NSWindow,
              let label = nativeWindows.first(where: { $0.value === closingWin })?.key else {
            return
        }
        if let wv = nativeWebViews.removeValue(forKey: label) {
            wv.loadHTMLString("", baseURL: nil)
            wv.removeFromSuperview()
        }
        nativeWindows.removeValue(forKey: label)

        if label.hasPrefix("sticky-note-") {
            let noteId = String(label.dropFirst("sticky-note-".count))
            RustIpcBridge.shared.invoke("sticky_close_note", args: ["id": noteId])
        } else {
            RustIpcBridge.shared.invoke("kernel_destroy_window", args: ["label": label])
        }
    }

    func windowDidResignKey(_ notification: Notification) {
        guard let blurredWin = notification.object as? NSWindow,
              let label = nativeWindows.first(where: { $0.value === blurredWin })?.key else {
            return
        }
        if label == "clipboard-popup" {
            RustIpcBridge.shared.invoke("kernel_blur_window", args: ["label": label]) { _ in
                self.syncWithRustKernel()
            }
        }
    }

    // MARK: - Quick Paste (`Cmd+V`) to Previous External App

    private func simulatePasteToPreviousExternalApp() {
        guard let targetApp = lastExternalApp else { return }
        NSApp.yieldActivation(to: targetApp)
        targetApp.activate()

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
            let src = CGEventSource(stateID: .hidSystemState)
            let vKeyCode: CGKeyCode = 0x09 // 'v'
            if let keyDown = CGEvent(keyboardEventSource: src, virtualKey: vKeyCode, keyDown: true),
               let keyUp = CGEvent(keyboardEventSource: src, virtualKey: vKeyCode, keyDown: false) {
                keyDown.flags = .maskCommand
                keyUp.flags = .maskCommand
                keyDown.post(tap: .cghidEventTap)
                keyUp.post(tap: .cghidEventTap)
            }
        }
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
