import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

// aurwatch bar widget: Arch icon + total -bin downloads. Turns the urgent
// color when a package needs attention (flagged out-of-date, a GitHub release
// that isn't packaged, orphaned) or the data is stale.
// Left click: the TUI. Right click: fetch now.
BarWidget {
  id: root
  moduleName: "custom.aurwatch"

  readonly property string pluginDir: decodeURIComponent(
    String(Qt.resolvedUrl("."))
      .replace(/^file:\/\//, "")
      .replace(/\/+$/, ""))
  readonly property string togglePath: pluginDir + "/aurwatch-toggle"

  // Nerd Font Arch Linux logo (nf-linux-archlinux).
  readonly property string icon: ""

  property var summary: null
  readonly property bool haveData: summary !== null && summary.have_data === true
  readonly property bool needsAttention: haveData && (summary.alerts > 0 || summary.stale)

  function age(secs) {
    if (secs < 60) return "just now"
    if (secs < 3600) return Math.floor(secs / 60) + "m ago"
    if (secs < 86400) return Math.floor(secs / 3600) + "h ago"
    return Math.floor(secs / 86400) + "d ago"
  }

  function tooltip() {
    if (summary && summary.missing_binary)
      return "aurwatch isn't installed — run install.sh in the aurwatch repo"
    if (!haveData)
      return "aurwatch — no data yet\nRight-click to fetch"
    var s = summary
    var head = s.packages + " AUR packages · ↓" + s.downloads + " downloads"
    if (s.downloads_7d !== null && s.downloads_7d !== undefined)
      head += " (" + s.downloads_7d + " this week)"
    head += " · ★" + s.votes
    var lines = [head]
    if (s.alerts > 0)
      lines.push("⚠ " + s.alerts + (s.alerts === 1 ? " package needs" : " packages need") + " attention")
    lines.push("")
    lines = lines.concat(s.lines)
    lines.push("")
    lines.push("Updated " + age(s.age_secs) + (s.stale ? " — stale, is the timer running?" : ""))
    lines.push("Click: open aurwatch · Right-click: fetch now")
    return lines.join("\n")
  }

  function parseSummary(raw) {
    try {
      var parsed = JSON.parse(raw)
      if (parsed && typeof parsed === "object") root.summary = parsed
    } catch (e) {
      // Keep the last good values rather than blanking on one bad poll.
    }
  }

  function poll() {
    if (!summaryProc.running) summaryProc.running = true
  }

  function launch(button) {
    if (!root.bar) return
    if (button === Qt.RightButton) {
      root.bar.run(Util.shellQuote(root.togglePath) + " --refresh")
      // A fetch takes a few seconds; check back for the new numbers.
      repoll.restart()
    } else {
      root.bar.run(Util.shellQuote(root.togglePath))
    }
  }

  Component.onCompleted: poll()

  // latest.json changes at most every few hours (timer) or on demand, and the
  // summary is a local file read, so a minute is plenty.
  Timer {
    interval: 60000
    running: true
    repeat: true
    onTriggered: root.poll()
  }

  // After a right-click refresh: poll a few times while the fetch finishes.
  Timer {
    id: repoll
    property int remaining: 0
    interval: 3000
    repeat: true
    onRunningChanged: if (running) remaining = 5
    onTriggered: {
      root.poll()
      if (--remaining <= 0) stop()
    }
  }

  Process {
    id: summaryProc
    command: [root.togglePath, "--summary"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.parseSummary(text)
    }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    bar: root.bar
    // A vertical bar has no room for the number; the tooltip still has it.
    text: root.haveData && !root.vertical
      ? (root.icon + " " + root.summary.downloads)
      : root.icon
    tooltipText: root.tooltip()
    active: root.needsAttention
    activeColor: Color.urgent
    dimmed: !root.haveData
    horizontalMargin: 6
    verticalPadding: 6
    fixedWidth: root.vertical ? root.barSize : -1
    fixedHeight: root.barSize
    onPressed: function(button) { root.launch(button) }
  }
}
