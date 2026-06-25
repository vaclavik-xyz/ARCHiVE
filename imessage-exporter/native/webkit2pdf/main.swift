// webkit2pdf — render a local HTML file to a paginated PDF using WebKit + Quartz.
//
// macOS native apps render chat transcripts with WebKit and let Apple's
// Quartz/CoreGraphics PDF engine write the file. Quartz emits far more compact
// content streams than Chrome's Skia engine, so the PDF is dramatically smaller
// for the same text-heavy content while keeping real (searchable) text and
// embedded fonts. WebKit also preserves embedded JPEGs instead of re-storing
// them losslessly the way Chrome does, so no image-recompression pass is needed.
//
// WebKit's NSPrintOperation path deadlocks in a faceless process, so we use
// WKWebView.createPDF, which works headless. createPDF captures one region per
// call, so we paginate ourselves: break pages at top-level message boundaries
// (never mid-bubble) into A4-proportioned slices, render each slice, and merge
// the slices with PDFKit.
//
// Usage:
//   webkit2pdf <input.html> <output.pdf> [rootDir]   render one HTML to PDF
//   webkit2pdf --merge <out.pdf> <in1.pdf> ...        concatenate page PDFs
//
// Set WK2PDF_TRACE=1 for progress on stderr.

import AppKit
import WebKit
import PDFKit

let traceEnabled = ProcessInfo.processInfo.environment["WK2PDF_TRACE"] != nil

func die(_ msg: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(("webkit2pdf: " + msg + "\n").data(using: .utf8)!)
    exit(code)
}
func trace(_ msg: String) {
    if traceEnabled { fputs("webkit2pdf: " + msg + "\n", stderr); fflush(stderr) }
}

let argv = CommandLine.arguments
guard argv.count >= 3 else {
    die("usage: webkit2pdf <input.html> <output.pdf> [rootDir]  |  webkit2pdf --merge <out.pdf> <in...>", 2)
}

// Merge mode: concatenate already-rendered page PDFs into one document.
if argv[1] == "--merge" {
    let out = URL(fileURLWithPath: argv[2])
    let merged = PDFDocument()
    for path in argv[3...] {
        guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else {
            die("could not open \(path) for merge", 9)
        }
        for i in 0..<doc.pageCount {
            if let page = doc.page(at: i) { merged.insert(page, at: merged.pageCount) }
        }
    }
    exit(merged.write(to: out) ? 0 : 8)
}

let inputURL = URL(fileURLWithPath: argv[1])
let outputURL = URL(fileURLWithPath: argv[2])
let rootURL = argv.count >= 4
    ? URL(fileURLWithPath: argv[3], isDirectory: true)
    : inputURL.deletingLastPathComponent()

// Layout width in CSS px (== PDF points 1:1 in createPDF); A4 aspect is 1.4142,
// so derive the slice height from the width to get A4-proportioned pages.
let pageWidth = 794.0
let pageHeight = (pageWidth * 841.89 / 595.22).rounded() // ~1123

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

final class Renderer: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    let outputURL: URL
    var pages: [(Double, Double)] = []
    var rendered: [Data] = []

    init(outputURL: URL) {
        let frame = NSRect(x: 0, y: 0, width: pageWidth, height: pageHeight)
        self.webView = WKWebView(frame: frame, configuration: WKWebViewConfiguration())
        self.outputURL = outputURL
        super.init()
        webView.navigationDelegate = self
    }

    func start(_ url: URL, _ root: URL) { webView.loadFileURL(url, allowingReadAccessTo: root) }

    func webView(_ wv: WKWebView, didFinish nav: WKNavigation!) {
        trace("loaded; measuring layout")
        measure(attempt: 0)
    }
    func webView(_ wv: WKWebView, didFail nav: WKNavigation!, withError e: Error) {
        die("navigation failed: \(e.localizedDescription)", 4)
    }
    func webView(_ wv: WKWebView, didFailProvisionalNavigation nav: WKNavigation!, withError e: Error) {
        die("load failed: \(e.localizedDescription)", 4)
    }

    // Wait for images, then read total height and every top-level message's top
    // offset so pages can break between messages rather than mid-bubble.
    func measure(attempt: Int) {
        let js = """
        (function(){
          var imgs = document.images;
          for (var i=0;i<imgs.length;i++){ if(!imgs[i].complete) return "WAIT"; }
          if (document.readyState !== 'complete') return "WAIT";
          var all = document.querySelectorAll('.message');
          var tops = [];
          for (var i=0;i<all.length;i++){
            var e = all[i];
            if (!e.parentElement || !e.parentElement.closest('.message')) {
              tops.push(Math.round(e.getBoundingClientRect().top + window.scrollY));
            }
          }
          return JSON.stringify({h: Math.ceil(document.documentElement.scrollHeight), tops: tops});
        })()
        """
        webView.evaluateJavaScript(js) { [weak self] result, _ in
            guard let self = self else { return }
            let s = (result as? String) ?? "WAIT"
            if s == "WAIT" {
                if attempt >= 1200 { die("page never finished loading", 6) }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { self.measure(attempt: attempt + 1) }
                return
            }
            guard let data = s.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let h = (obj["h"] as? NSNumber)?.doubleValue else {
                die("could not parse layout metrics", 7)
            }
            let tops = (obj["tops"] as? [NSNumber])?.map { $0.doubleValue } ?? []
            self.pages = Renderer.paginate(totalHeight: h, messageTops: tops, pageHeight: pageHeight)
            trace("height=\(Int(h)) messages=\(tops.count) pages=\(self.pages.count)")
            self.renderPage(0)
        }
    }

    // Pack messages into pages no taller than `pageHeight`, breaking before the
    // message that would overflow. The break only fires once the page already
    // holds a message, so the leading margin never becomes its own near-blank
    // page and a message taller than a page becomes its own over-tall page
    // instead of being cut mid-bubble.
    static func paginate(totalHeight: Double, messageTops: [Double], pageHeight: Double) -> [(Double, Double)] {
        if messageTops.isEmpty || totalHeight <= pageHeight { return [(0, totalHeight)] }

        var ranges: [(Double, Double)] = []
        var pageStart = 0.0
        var lastTop = 0.0       // top of the most recent message on the current page
        var placed = false      // page already holds at least one message
        for top in messageTops.sorted() {
            if placed && top - pageStart > pageHeight {
                ranges.append((pageStart, top))
                pageStart = top
            }
            lastTop = top
            placed = true
        }
        // The trailing segment (last message to the bottom) has no boundary
        // after it, so check it explicitly: if it overflows and the page holds
        // more than that last message, give the last message its own page.
        if placed && totalHeight - pageStart > pageHeight && lastTop > pageStart {
            ranges.append((pageStart, lastTop))
            pageStart = lastTop
        }
        ranges.append((pageStart, totalHeight))
        return ranges
    }

    func renderPage(_ idx: Int) {
        if idx >= pages.count { finish(); return }
        let (y0, y1) = pages[idx]
        let cfg = WKPDFConfiguration()
        cfg.rect = CGRect(x: 0, y: y0, width: pageWidth, height: y1 - y0)
        webView.createPDF(configuration: cfg) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let data):
                self.rendered.append(data)
                if idx % 25 == 0 { trace("rendered page \(idx + 1)/\(self.pages.count)") }
                self.renderPage(idx + 1)
            case .failure(let e):
                die("createPDF failed on page \(idx): \(e.localizedDescription)", 5)
            }
        }
    }

    func finish() {
        trace("merging \(rendered.count) pages")
        let merged = PDFDocument()
        for data in rendered {
            guard let doc = PDFDocument(data: data) else { continue }
            for i in 0..<doc.pageCount {
                if let page = doc.page(at: i) { merged.insert(page, at: merged.pageCount) }
            }
        }
        if merged.write(to: outputURL) {
            trace("wrote \(merged.pageCount) pages -> \(outputURL.path)")
            exit(0)
        }
        die("could not write \(outputURL.path)", 8)
    }
}

let renderer = Renderer(outputURL: outputURL)
// Backstop so a wedged render can never hold the process open forever.
DispatchQueue.main.asyncAfter(deadline: .now() + 1800) { die("timed out after 1800s", 6) }
trace("loading \(inputURL.path)")
renderer.start(inputURL, rootURL)
app.run()
