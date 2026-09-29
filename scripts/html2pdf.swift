// Renders an HTML file to a paginated US-Letter PDF using WebKit's print
// pipeline (CoreGraphics PDF output with subset embedded fonts — close to
// what browsers and many resume builders produce).
//
// usage: swift scripts/html2pdf.swift input.html output.pdf
import AppKit
import WebKit

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write("usage: html2pdf input.html output.pdf\n".data(using: .utf8)!)
    exit(2)
}
let input = URL(fileURLWithPath: args[1])
let output = URL(fileURLWithPath: args[2])

final class Loader: NSObject, WKNavigationDelegate {
    let web: WKWebView
    init(web: WKWebView) { self.web = web }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let info = NSPrintInfo()
        info.paperSize = NSSize(width: 612, height: 792)
        info.topMargin = 0
        info.bottomMargin = 0
        info.leftMargin = 0
        info.rightMargin = 0
        info.horizontalPagination = .fit
        info.verticalPagination = .automatic
        info.jobDisposition = .save
        info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = output
        let op = webView.printOperation(with: info)
        op.showsPrintPanel = false
        op.showsProgressPanel = false
        op.view?.frame = NSRect(x: 0, y: 0, width: 612, height: 792)
        op.runModal(for: NSWindow(), delegate: self, didRun: #selector(done), contextInfo: nil)
    }

    @objc func done() { exit(0) }
}

let app = NSApplication.shared
let web = WKWebView(frame: NSRect(x: 0, y: 0, width: 612, height: 792))
let loader = Loader(web: web)
web.navigationDelegate = loader
web.loadFileURL(input, allowingReadAccessTo: input.deletingLastPathComponent())
DispatchQueue.main.asyncAfter(deadline: .now() + 20) {
    FileHandle.standardError.write("timeout\n".data(using: .utf8)!)
    exit(1)
}
app.run()
