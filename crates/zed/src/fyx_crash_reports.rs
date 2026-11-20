//! Plan 6.4: local crash reports. Fyx keeps Zed's crash handler (a `--crash-handler` child that
//! writes a minidump and a JSON summary per crashed session) but never uploads: the files go to
//! `<data dir>/crashes`, the "Crash reports" list shows them, and "Report" opens a prefilled
//! GitHub issue in the browser. The minidump stays on the computer unless the user attaches it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui::{
    App, AppContext as _, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Task,
    WeakEntity, actions,
};
use picker::{Picker, PickerDelegate};
use release_channel::ReleaseChannel;
use ui::{ListItem, ListItemSpacing, prelude::*};
use util::ResultExt as _;
use workspace::notifications::simple_message_notification::MessageNotification;
use workspace::notifications::{NotificationId, show_app_notification};
use workspace::{ModalView, Workspace};

actions!(
    fyx,
    [
        /// Lists the crash reports Fyx wrote on this computer, to report one on GitHub.
        OpenCrashReports
    ]
);

const NEW_ISSUE_URL: &str = "https://github.com/shelenhovskyi/fyx/issues/new";
/// Older reports are deleted when the list is read.
const KEEP_REPORTS: usize = 20;
/// A crash older than this is listed but not announced at startup.
const ANNOUNCE_WITHIN: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Next to the crash handler's socket: the folder the handler writes into. The handler is a
/// child started with only the socket path, so it cannot see `--user-data-dir` otherwise.
const SIDECAR_EXTENSION: &str = "fyx-crash-dir";
const LAST_SEEN_FILE: &str = "last-seen";
/// Keeps the issue URL well under the length browsers and GitHub accept.
const MAX_MESSAGE_CHARS: usize = 1500;

/// Where crash reports are written and read.
pub fn crash_dir() -> PathBuf {
    paths::data_dir().join("crashes")
}

/// Whether to start the crash handler. `FYX_CRASH_REPORTS=0` turns it off and `=1` on; else on,
/// except in debug builds of the dev channel (`cargo run`), where a panic should print its
/// backtrace instead. Upstream starts it only with an upload endpoint, which Fyx never has.
pub fn should_install_crash_handler(channel: ReleaseChannel) -> bool {
    install_decision(
        std::env::var("FYX_CRASH_REPORTS").ok().as_deref(),
        client::telemetry::should_install_crash_handler(channel),
        cfg!(debug_assertions) && channel == ReleaseChannel::Dev,
    )
}

fn install_decision(setting: Option<&str>, upstream: bool, debug_dev: bool) -> bool {
    match setting {
        Some("0" | "false") => false,
        Some("1" | "true") => true,
        _ => upstream || !debug_dev,
    }
}

/// The crash handler's socket for process `pid`, with the crash folder recorded beside it.
pub fn socket_path(pid: u32) -> PathBuf {
    let socket = paths::temp_dir().join(format!("zed-crash-handler-{pid}"));
    record_crash_dir(&socket, &crash_dir());
    socket
}

fn record_crash_dir(socket: &Path, dir: &Path) {
    let Some(text) = dir.to_str() else {
        log::warn!("crash reports: {dir:?} is not UTF-8; the handler writes to the logs dir");
        return;
    };
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(sidecar(socket), text))
        .log_err();
}

/// For `fyx --crash-handler <socket>`: the folder the parent recorded, else the logs dir.
pub fn handler_dir(socket: &Path) -> PathBuf {
    recorded_crash_dir(socket).unwrap_or_else(|| paths::logs_dir().clone())
}

fn recorded_crash_dir(socket: &Path) -> Option<PathBuf> {
    let sidecar = sidecar(socket);
    let text = std::fs::read_to_string(&sidecar).ok()?;
    std::fs::remove_file(&sidecar).log_err();
    let dir = PathBuf::from(text.trim());
    std::fs::create_dir_all(&dir).log_err()?;
    Some(dir)
}

fn sidecar(socket: &Path) -> PathBuf {
    socket.with_extension(SIDECAR_EXTENSION)
}

/// One crashed session, from its `<session>.json` summary.
#[derive(Clone, Debug, PartialEq)]
pub struct CrashReport {
    pub session_id: String,
    pub version: String,
    pub channel: String,
    pub commit: String,
    /// The panic message and its `file:line`, for a panic.
    pub panic: Option<(String, String)>,
    pub abort_message: Option<String>,
    pub minidump_error: Option<String>,
    pub summary: PathBuf,
    /// `<session>.dmp`, when the handler wrote one.
    pub minidump: Option<PathBuf>,
    pub written: SystemTime,
}

impl CrashReport {
    fn parse(summary: &Path, bytes: &[u8], written: SystemTime) -> Option<Self> {
        let info: crashes::CrashInfo = serde_json::from_slice(bytes).log_err()?;
        let minidump = summary.with_extension("dmp");
        Some(Self {
            session_id: info.init.session_id,
            version: info.init.zed_version,
            channel: info.init.release_channel,
            commit: info.init.commit_sha,
            panic: info.panic.map(|panic| (panic.message, panic.span)),
            abort_message: info.abort_message,
            minidump_error: info.minidump_error,
            minidump: minidump.is_file().then_some(minidump),
            summary: summary.to_path_buf(),
            written,
        })
    }

    /// One line for the list and the issue title.
    pub fn headline(&self) -> String {
        let text = match (&self.panic, &self.abort_message) {
            (Some((message, _)), _) => message.lines().next().unwrap_or_default().to_owned(),
            (None, Some(abort)) => abort.clone(),
            (None, None) => "native crash".to_owned(),
        };
        truncate(&text, 80)
    }

    /// The prefilled "New issue" page. Nothing leaves the computer until the user submits it.
    pub fn issue_url(&self) -> String {
        let title = format!("Crash: {}", self.headline());
        let short_commit = self.commit.get(..10).unwrap_or(&self.commit);
        let mut body = format!(
            "### What happened\n\n<!-- What were you doing when Fyx crashed? -->\n\n\
             ### Crash\n\n- Fyx {} ({}, {short_commit})\n- {} {}\n",
            self.version,
            self.channel,
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        match &self.panic {
            Some((message, span)) => {
                body.push_str(&format!(
                    "- Panic at `{span}`:\n\n```\n{}\n```\n",
                    truncate(message, MAX_MESSAGE_CHARS).replace("```", "'''")
                ));
            }
            None => body.push_str("- A native crash (no panic message)\n"),
        }
        if let Some(abort) = &self.abort_message {
            body.push_str(&format!("- Abort message: `{}`\n", truncate(abort, 300)));
        }
        if let Some(error) = &self.minidump_error {
            body.push_str(&format!("- Minidump error: `{}`\n", truncate(error, 300)));
        }
        if self.minidump.is_some() {
            body.push_str(&format!(
                "\nThe minidump `{}.dmp` stays on my computer. (Attach it only if you are fine \
                 sharing it: it can hold parts of memory, such as open files.)\n",
                self.session_id
            ));
        }
        match url::Url::parse_with_params(NEW_ISSUE_URL, &[("title", title), ("body", body)]) {
            Ok(url) => url.to_string(),
            Err(_) => NEW_ISSUE_URL.to_owned(),
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The reports in `dir`, newest first. Deletes all but the newest [`KEEP_REPORTS`].
pub fn read_reports(dir: &Path) -> Vec<CrashReport> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut reports: Vec<CrashReport> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return None;
            }
            let written = std::fs::metadata(&path).ok()?.modified().ok()?;
            let bytes = std::fs::read(&path).ok()?;
            CrashReport::parse(&path, &bytes, written)
        })
        .collect();
    reports.sort_by_key(|report| std::cmp::Reverse(report.written));
    for old in reports.iter().skip(KEEP_REPORTS) {
        std::fs::remove_file(&old.summary).log_err();
        if let Some(minidump) = &old.minidump {
            std::fs::remove_file(minidump).log_err();
        }
    }
    reports.truncate(KEEP_REPORTS);
    reports
}

/// The newest report, when it is recent and was not announced before; marks it announced.
fn unannounced(dir: &Path, reports: &[CrashReport], now: SystemTime) -> Option<CrashReport> {
    let newest = reports.first()?;
    let last_seen = dir.join(LAST_SEEN_FILE);
    if std::fs::read_to_string(&last_seen).is_ok_and(|seen| seen.trim() == newest.session_id) {
        return None;
    }
    std::fs::write(&last_seen, &newest.session_id).log_err();
    let recent = now
        .duration_since(newest.written)
        .is_ok_and(|age| age <= ANNOUNCE_WITHIN);
    recent.then(|| newest.clone())
}

/// After the workspace is initialized: the action, and a notification for a crash since the last
/// start.
pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &OpenCrashReports, window, cx| {
            let reports = read_reports(&crash_dir());
            workspace.toggle_modal(window, cx, move |window, cx| {
                CrashReports::new(reports, window, cx)
            });
        });
    })
    .detach();

    cx.spawn(async move |cx| {
        let report = cx
            .background_spawn(async move {
                let dir = crash_dir();
                let reports = read_reports(&dir);
                unannounced(&dir, &reports, SystemTime::now())
            })
            .await;
        let Some(report) = report else {
            return;
        };
        cx.update(|cx| announce(report, cx));
    })
    .detach();
}

fn announce(report: CrashReport, cx: &mut App) {
    struct CrashedLastTime;
    show_app_notification(NotificationId::unique::<CrashedLastTime>(), cx, move |cx| {
        let url = report.issue_url();
        cx.new(|cx| {
            MessageNotification::new(
                format!(
                    "Fyx crashed last time: {}\nThe report stays on this computer.",
                    report.headline()
                ),
                cx,
            )
            .primary_message("Report on GitHub")
            .primary_icon(IconName::Github)
            .primary_on_click(move |_, cx| {
                cx.open_url(&url);
                cx.emit(DismissEvent);
            })
            .secondary_message("Crash Reports")
            .secondary_on_click(|window, cx| {
                window.dispatch_action(Box::new(OpenCrashReports), cx);
                cx.emit(DismissEvent);
            })
        })
    });
}

/// The "Crash reports" list: Enter reports one on GitHub, secondary confirm shows its files.
pub struct CrashReports {
    picker: Entity<Picker<CrashReportsDelegate>>,
}

impl CrashReports {
    fn new(reports: Vec<CrashReport>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = CrashReportsDelegate {
            list: cx.entity().downgrade(),
            reports,
            selected_index: 0,
        };
        let picker = cx.new(|cx| Picker::nonsearchable_uniform_list(delegate, window, cx));
        Self { picker }
    }
}

impl Render for CrashReports {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().w(rems(40.)).child(self.picker.clone())
    }
}

impl Focusable for CrashReports {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl EventEmitter<DismissEvent> for CrashReports {}
impl ModalView for CrashReports {}

pub struct CrashReportsDelegate {
    list: WeakEntity<CrashReports>,
    reports: Vec<CrashReport>,
    selected_index: usize,
}

impl PickerDelegate for CrashReportsDelegate {
    type ListItem = ListItem;

    fn name() -> &'static str {
        "crash reports"
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> std::sync::Arc<str> {
        "Crash reports: Enter opens a GitHub issue to fill in, the secondary key shows the files"
            .into()
    }

    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        Some("No crash reports on this computer".into())
    }

    fn match_count(&self) -> usize {
        self.reports.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        index: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = index;
    }

    fn update_matches(
        &mut self,
        _query: String,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        Task::ready(())
    }

    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(report) = self.reports.get(self.selected_index) {
            if secondary {
                cx.reveal_path(report.minidump.as_ref().unwrap_or(&report.summary));
            } else {
                cx.open_url(&report.issue_url());
            }
        }
        self.dismissed(window, cx);
    }

    fn dismissed(&mut self, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.list.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
    }

    fn render_match(
        &self,
        index: usize,
        selected: bool,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let report = self.reports.get(index)?;
        let when = chrono::DateTime::<chrono::Local>::from(report.written)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        Some(
            ListItem::new(index)
                .inset(true)
                .spacing(ListItemSpacing::Sparse)
                .toggle_state(selected)
                .child(
                    h_flex()
                        .gap_2()
                        .child(Label::new(when).color(Color::Muted))
                        .child(Label::new(report.headline()).truncate()),
                )
                .end_slot(
                    Label::new(format!("{} {}", report.version, report.channel))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system temp dir, removed on drop (the zed crate has no
    /// `tempfile` for its tests).
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("fyx-crash-reports-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&path).ok();
            assert!(std::fs::create_dir_all(&path).is_ok());
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn summary(session: &str, panic: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "init": {
                "session_id": session,
                "zed_version": "0.9.0",
                "binary": "zed",
                "release_channel": "dev",
                "commit_sha": "0123456789abcdef",
            },
            "panic": panic.map(|message| serde_json::json!({
                "message": message,
                "span": "crates/x/src/x.rs:7",
            })),
            "minidump_error": null,
            "gpus": [],
            "active_gpu": null,
        })
    }

    fn write_report(dir: &Path, session: &str, panic: Option<&str>, age: Duration) {
        let path = dir.join(session).with_extension("json");
        let bytes = serde_json::to_vec(&summary(session, panic)).unwrap_or_default();
        assert!(std::fs::write(&path, bytes).is_ok());
        let file = std::fs::File::options().write(true).open(&path);
        let written = SystemTime::now() - age;
        assert!(file.and_then(|file| file.set_modified(written)).is_ok());
    }

    #[test]
    fn reports_are_listed_newest_first_and_pruned() {
        let dir = TestDir::new("prune");
        for index in 0..KEEP_REPORTS + 3 {
            let age = Duration::from_secs(60 * (index as u64 + 1));
            write_report(dir.path(), &format!("s{index}"), Some("boom"), age);
        }
        assert!(std::fs::write(dir.path().join("s0.dmp"), b"dump").is_ok());
        assert!(std::fs::write(dir.path().join("broken.json"), b"{").is_ok());
        let reports = read_reports(dir.path());
        assert_eq!(reports.len(), KEEP_REPORTS);
        assert_eq!(reports[0].session_id, "s0");
        assert_eq!(reports[0].minidump, Some(dir.path().join("s0.dmp")));
        assert!(
            !dir.path()
                .join(format!("s{}.json", KEEP_REPORTS + 2))
                .exists(),
            "the oldest are deleted"
        );
    }

    #[test]
    fn the_issue_is_prefilled_and_names_no_path() {
        let dir = TestDir::new("issue");
        write_report(
            dir.path(),
            "abc",
            Some("index out of bounds\nsecond line"),
            Duration::ZERO,
        );
        assert!(std::fs::write(dir.path().join("abc.dmp"), b"dump").is_ok());
        let reports = read_reports(dir.path());
        let Some(report) = reports.first() else {
            panic!("no report");
        };
        assert_eq!(report.headline(), "index out of bounds");
        let url = report.issue_url();
        assert!(url.starts_with("https://github.com/shelenhovskyi/fyx/issues/new?title=Crash"));
        let Ok(parsed) = url::Url::parse(&url) else {
            panic!("bad url {url}");
        };
        let body = parsed
            .query_pairs()
            .find(|(key, _)| key == "body")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        assert!(body.contains("Fyx 0.9.0 (dev, 0123456789)"), "{body}");
        assert!(body.contains("crates/x/src/x.rs:7"), "{body}");
        assert!(body.contains("`abc.dmp` stays on my computer"), "{body}");
        let folder = dir.path().to_string_lossy().into_owned();
        assert!(!body.contains(&folder), "no local path in the issue");

        let long = "x".repeat(MAX_MESSAGE_CHARS * 4);
        let mut huge = report.clone();
        huge.panic = Some((long, String::new()));
        assert!(huge.issue_url().len() < 8000);
    }

    #[test]
    fn a_crash_is_announced_once_and_only_when_recent() {
        let dir = TestDir::new("announce");
        write_report(dir.path(), "new", None, Duration::from_secs(60));
        let reports = read_reports(dir.path());
        let now = SystemTime::now();
        assert_eq!(
            unannounced(dir.path(), &reports, now).map(|report| report.session_id),
            Some("new".to_owned())
        );
        assert_eq!(unannounced(dir.path(), &reports, now), None, "only once");

        let other = TestDir::new("announce-old");
        write_report(other.path(), "old", None, ANNOUNCE_WITHIN * 2);
        let reports = read_reports(other.path());
        assert_eq!(unannounced(other.path(), &reports, now), None, "too old");
    }

    #[test]
    fn the_handler_learns_the_folder_from_the_sidecar() {
        let dir = TestDir::new("sidecar");
        let socket = dir.path().join("zed-crash-handler-42");
        let crashes = dir.path().join("profile").join("crashes");
        record_crash_dir(&socket, &crashes);
        assert!(crashes.is_dir());
        assert_eq!(recorded_crash_dir(&socket), Some(crashes));
        assert_eq!(recorded_crash_dir(&socket), None, "read once");
    }

    #[test]
    fn the_handler_is_on_except_for_cargo_run() {
        assert!(install_decision(None, false, false));
        assert!(!install_decision(None, false, true));
        assert!(install_decision(None, true, true));
        assert!(!install_decision(Some("0"), true, false));
        assert!(install_decision(Some("1"), false, true));
    }
}
