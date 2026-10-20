//! Local, optional picker decoration. GTK objects stay on the UI thread; audio
//! identities and capture never depend on a desktop entry or an icon lookup.
use super::applications::{ApplicationIconBudget, ApplicationSnapshot};
use gtk::{gio, prelude::*, IconLookupFlags, IconTheme};
use std::cell::LazyCell;
use std::collections::HashMap;

const ICON_SIZE: i32 = 32;
const MAX_ICON_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Default, Debug, PartialEq, Eq, Hash)]
pub(super) struct IconSource {
    icon_names: Vec<String>,
    application_id: Option<String>,
    binary: Option<String>,
    pub(super) name: Option<String>,
}

impl IconSource {
    pub(super) fn from_properties(properties: &libpulse_binding::proplist::Proplist) -> Self {
        let read = |key| {
            properties
                .get_str(key)
                .and_then(|value| bounded_hint(&value))
        };
        Self {
            icon_names: ["application.icon_name", "media.icon_name"]
                .into_iter()
                .filter_map(read)
                .filter(|name| theme_name(name))
                .collect(),
            application_id: read("application.id").filter(|id| theme_name(id)),
            binary: read("application.process.binary"),
            name: super::linux_streams::application_name(properties),
        }
    }
}

fn bounded_hint(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

// Stream properties supply theme names, never paths, serialized GIcons or URLs.
fn theme_name(value: &str) -> bool {
    value != "." && value != ".." && !value.contains(['/', '\\', ':'])
}

pub(crate) async fn populate(
    app: &tauri::AppHandle,
    mut snapshot: ApplicationSnapshot,
) -> ApplicationSnapshot {
    if snapshot.applications.is_empty() {
        return snapshot;
    }
    let sources = super::linux_streams::icon_sources(&snapshot.applications);
    let (reply, response) = tokio::sync::oneshot::channel();
    // No libpulse objects or registry guards cross into the GTK closure.
    if app
        .run_on_main_thread(move || {
            if reply.is_closed() {
                return;
            }
            let icons = IconTheme::default()
                .map(|theme| encode_sources(&theme, &sources, gio::AppInfo::all))
                .unwrap_or_default();
            let _ = reply.send(icons);
        })
        .is_err()
    {
        return snapshot;
    }
    if let Ok(Ok(icons)) = tokio::time::timeout(std::time::Duration::from_secs(5), response).await {
        for (application, icon) in snapshot.applications.iter_mut().zip(icons) {
            application.icon_data_url = icon;
        }
    }
    snapshot
}

fn encode_sources(
    theme: &IconTheme,
    sources: &[Option<IconSource>],
    installed: impl FnOnce() -> Vec<gio::AppInfo>,
) -> Vec<Option<String>> {
    let mut budget = ApplicationIconBudget::default();
    // Only this bounded request caches decoding, including misses. Several
    // streams of the same app keep distinct IDs but share the same artwork.
    let mut decoded: HashMap<&IconSource, Option<Vec<u8>>> = HashMap::new();
    let desktops = LazyCell::new(|| {
        let mut apps = installed();
        apps.sort_by_key(|app| app.id());
        apps.truncate(4096);
        apps
    });
    sources
        .iter()
        .map(|source| {
            let source = source.as_ref()?;
            let png = decoded.entry(source).or_insert_with(|| {
                source
                    .icon_names
                    .iter()
                    .find_map(|name| {
                        theme
                            .lookup_icon(name, ICON_SIZE, IconLookupFlags::FORCE_SIZE)
                            .and_then(icon_png)
                    })
                    .or_else(|| {
                        if source.application_id.is_none()
                            && source.binary.is_none()
                            && source.name.is_none()
                        {
                            return None;
                        }
                        let icon = desktop_icon(source, &desktops)?;
                        // Installed desktop metadata may use a local absolute
                        // FileIcon; reject all remote files and other GIcon types.
                        if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
                            if !file.file().path().is_some_and(|path| path.is_absolute()) {
                                return None;
                            }
                        } else if !icon.is::<gio::ThemedIcon>() {
                            return None;
                        }
                        theme
                            .lookup_by_gicon(&icon, ICON_SIZE, IconLookupFlags::FORCE_SIZE)
                            .and_then(icon_png)
                    })
            });
            budget.encode_png(png.as_ref()?)
        })
        .collect()
}

fn icon_png(info: gtk::IconInfo) -> Option<Vec<u8>> {
    if let Some(path) = info.filename() {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_ICON_FILE_BYTES {
            return None;
        }
    }
    let pixbuf = info.load_icon().ok()?;
    let pixbuf =
        pixbuf.scale_simple(ICON_SIZE, ICON_SIZE, gtk::gdk_pixbuf::InterpType::Bilinear)?;
    let png = pixbuf.save_to_bufferv("png", &[]).ok()?;
    (png.len() <= super::applications::MAX_APPLICATION_ICON_PNG_BYTES).then_some(png)
}

// A launcher can add a variant suffix while its audio process keeps the base
// product name. Require a word boundary; "Brave" must not match "Bravely".
fn variant_name(candidate: &str, base: &str) -> bool {
    candidate
        .get(..base.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(base))
        && candidate.get(base.len()..).is_some_and(|suffix| {
            suffix.starts_with([' ', '-']) && !suffix.trim_matches([' ', '-']).is_empty()
        })
}

// Exported Flatpak launchers run `flatpak`, not the application's audio binary.
// Parse their declared command without running the launcher or inspecting PIDs
// (the audio process ID can belong to the sandbox's PID namespace).
fn flatpak_command(app: &gio::AppInfo) -> Option<std::ffi::OsString> {
    let id = app
        .downcast_ref::<gio::DesktopAppInfo>()?
        .string("X-Flatpak")?;
    let desktop_id = app.id()?;
    let desktop_id = desktop_id.strip_suffix(".desktop")?;
    // Flatpak also exports ID.foo.desktop and ID-foo.desktop launchers.
    if desktop_id != id.as_str()
        && !desktop_id
            .strip_prefix(id.as_str())
            .is_some_and(|suffix| suffix.starts_with(['.', '-']) && suffix.len() > 1)
    {
        return None;
    }
    let arguments = gtk::glib::shell_parse_argv(app.commandline()?).ok()?;
    if std::path::Path::new(arguments.first()?).file_name()? != "flatpak"
        || arguments.get(1)? != "run"
        || !arguments.iter().any(|argument| argument == id.as_str())
    {
        return None;
    }
    arguments
        .iter()
        .enumerate()
        .skip(2)
        .find_map(|(index, argument)| {
            let argument = argument.to_str()?;
            if argument == "--command" {
                arguments.get(index + 1).cloned()
            } else {
                argument.strip_prefix("--command=").map(Into::into)
            }
        })
}

fn unique_icon<'a>(mut apps: impl Iterator<Item = &'a gio::AppInfo>) -> Option<gio::Icon> {
    let app = apps.next()?;
    // Ambiguous fallbacks need an exact hint, not a guess based on list order.
    if apps.next().is_some() {
        return None;
    }
    app.icon()
}

fn desktop_icon(source: &IconSource, desktops: &[gio::AppInfo]) -> Option<gio::Icon> {
    let id_matches = |app: &gio::AppInfo, value: &str| {
        app.id().is_some_and(|id| {
            id.trim_end_matches(".desktop")
                .eq_ignore_ascii_case(value.trim_end_matches(".desktop"))
        })
    };
    let binary_matches = |app: &gio::AppInfo, value: &str| {
        // D-Bus-only desktop entries legitimately have no Exec. gio 0.18's
        // executable binding is non-nullable, so guard it with commandline.
        if app.commandline().is_none() {
            return false;
        }
        app.executable().file_name().is_some_and(|binary| {
            std::path::Path::new(value)
                .file_name()
                .is_some_and(|value| binary == value)
        })
    };
    let executable_matches = |app: &gio::AppInfo, value: &str| {
        binary_matches(app, value)
            || flatpak_command(app).is_some_and(|command| {
                std::path::Path::new(&command).file_name()
                    == std::path::Path::new(value).file_name()
            })
    };
    if let Some(app) = source
        .application_id
        .as_deref()
        .and_then(|id| desktops.iter().find(|app| id_matches(app, id)))
    {
        return app.icon();
    }
    if let Some(binary) = source.binary.as_deref() {
        // Exec identifies a program, not a launcher: profiles, interpreters and
        // native/Flatpak installations can share it. Never pick by list order.
        let candidates: Vec<_> = desktops
            .iter()
            .filter(|app| executable_matches(app, binary))
            .collect();
        if !candidates.is_empty() {
            if let Some(name) = source.name.as_deref() {
                if let Some(app) = candidates.iter().find(|app| id_matches(app, name)) {
                    return app.icon();
                }
                let mut named = candidates
                    .iter()
                    .copied()
                    .filter(|app| app.name().eq_ignore_ascii_case(name))
                    .peekable();
                if named.peek().is_some() {
                    return unique_icon(named);
                }
            }
            return unique_icon(candidates.into_iter());
        }
    }
    let name = source.name.as_deref()?;
    if let Some(app) = desktops.iter().find(|app| id_matches(app, name)) {
        return app.icon();
    }
    let mut named = desktops
        .iter()
        .filter(|app| app.name().eq_ignore_ascii_case(name))
        .peekable();
    if named.peek().is_some() {
        // An ambiguous exact name must not fall through to a weaker variant.
        return unique_icon(named);
    }
    let mut binaries = desktops
        .iter()
        .filter(|app| executable_matches(app, name))
        .peekable();
    if binaries.peek().is_some() {
        return unique_icon(binaries);
    }
    unique_icon(
        desktops
            .iter()
            .filter(|app| variant_name(&app.name(), name)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_entry(
        directory: &std::path::Path,
        id: &str,
        name: &str,
        binary: &str,
        icon: &str,
    ) -> gio::AppInfo {
        let path = directory.join(format!("{id}.desktop"));
        std::fs::write(
            &path,
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec={binary}\nIcon={icon}\n"),
        )
        .unwrap();
        gio::DesktopAppInfo::from_filename(path).unwrap().upcast()
    }

    fn flatpak_desktop(
        directory: &std::path::Path,
        id: &str,
        options: &str,
        icon: &str,
    ) -> gio::AppInfo {
        flatpak_desktop_with_id(directory, id, id, options, icon)
    }

    fn flatpak_desktop_with_id(
        directory: &std::path::Path,
        desktop_id: &str,
        app_id: &str,
        options: &str,
        icon: &str,
    ) -> gio::AppInfo {
        // An existing stub launcher keeps fixtures independent of Flatpak being
        // installed on CI. It is never executed by an icon lookup.
        let launcher = directory.join("flatpak");
        if !launcher.exists() {
            std::os::unix::fs::symlink("/bin/true", &launcher).unwrap();
        }
        let path = directory.join(format!("{desktop_id}.desktop"));
        std::fs::write(&path, format!(
            "[Desktop Entry]\nType=Application\nName=腾讯会议\nExec=\"{}\" run {options} {app_id} @@u %u @@\nIcon={icon}\nX-Flatpak={app_id}\n",
            launcher.display()
        )).unwrap();
        gio::DesktopAppInfo::from_filename(path).unwrap().upcast()
    }

    #[test]
    #[ignore = "metadata/icon only; requires a display and MIMI_TEST_PLAYBACK_NODE on the desktop server"]
    fn live_playback_icon_is_available_without_capture() {
        let expected = std::env::var("MIMI_TEST_PLAYBACK_NODE").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let snapshot = runtime
            .block_on(super::super::linux_streams::list("en".into()))
            .unwrap();
        let selected = snapshot
            .applications
            .iter()
            .find(|application| application.name.starts_with(&format!("{expected} · ")))
            .expect("the expected playback node must be present");
        let sources = super::super::linux_streams::icon_sources(std::slice::from_ref(selected));
        gtk::init().unwrap();
        let icons = encode_sources(&IconTheme::default().unwrap(), &sources, gio::AppInfo::all);
        assert!(icons[0]
            .as_ref()
            .is_some_and(|icon| icon.starts_with("data:image/png;base64,")));
        eprintln!("linux live metadata iconAvailable=true capture=false");
    }

    #[test]
    fn stream_hints_are_bounded_local_names_and_preserve_application_identity() {
        let mut properties = libpulse_binding::proplist::Proplist::new().unwrap();
        for (key, value) in [
            ("application.icon_name", "https://example.invalid/icon.png"),
            ("media.icon_name", "player"),
            ("application.id", "org.example.Player"),
            ("application.process.binary", "/opt/player/bin/player"),
            ("node.name", "Native Player"),
        ] {
            properties.set_str(key, value).unwrap();
        }
        let hints = IconSource::from_properties(&properties);
        assert_eq!(hints.icon_names, ["player"]);
        assert_eq!(hints.name.as_deref(), Some("Native Player"));
        assert_eq!(hints.application_id.as_deref(), Some("org.example.Player"));
        assert_eq!(hints.binary.as_deref(), Some("/opt/player/bin/player"));
        assert!(bounded_hint(&"a".repeat(257)).is_none());
        assert!(bounded_hint("player\0").is_none());
        for invalid in ["/tmp/icon.png", "../icon", "file:///tmp/icon", ".."] {
            assert!(!theme_name(invalid));
        }
        properties
            .set_str("application.name", "Pulse Player")
            .unwrap();
        assert_eq!(
            IconSource::from_properties(&properties).name.as_deref(),
            Some("Pulse Player")
        );
        properties
            .set_str("application.name", " \nPulse Player\t ")
            .unwrap();
        assert_eq!(
            IconSource::from_properties(&properties).name.as_deref(),
            Some("Pulse Player")
        );
        properties
            .set_str("application.name", &"名".repeat(100))
            .unwrap();
        assert_eq!(
            IconSource::from_properties(&properties).name.unwrap().len(),
            99
        );
    }

    #[test]
    fn desktop_matching_uses_executable_and_native_name_without_launching() {
        let keyfile = gtk::glib::KeyFile::new();
        keyfile.load_from_data("[Desktop Entry]\nType=Application\nName=Native Player\nExec=/bin/true\nIcon=fixture-player\n", gtk::glib::KeyFileFlags::NONE).unwrap();
        let app: gio::AppInfo = gio::DesktopAppInfo::from_keyfile(&keyfile)
            .unwrap()
            .upcast();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("org.example.Background.desktop");
        std::fs::write(&path, "[Desktop Entry]\nType=Application\nName=Background\nDBusActivatable=true\nIcon=background\n").unwrap();
        let background = gio::DesktopAppInfo::from_filename(path).unwrap().upcast();
        let apps = [background, app];
        for source in [
            IconSource {
                binary: Some("/bin/true".into()),
                ..Default::default()
            },
            IconSource {
                name: Some("native player".into()),
                ..Default::default()
            },
            IconSource {
                name: Some("true".into()),
                ..Default::default()
            },
        ] {
            assert!(desktop_icon(&source, &apps).is_some());
        }
        assert!(desktop_icon(
            &IconSource {
                name: Some("Unrelated".into()),
                ..Default::default()
            },
            &apps
        )
        .is_none());
    }

    #[test]
    fn shared_executables_use_exact_names_without_depending_on_desktop_order() {
        let directory = tempfile::tempdir().unwrap();
        let work = desktop_entry(
            directory.path(),
            "aaa-work",
            "Work Profile",
            "/bin/true",
            "work",
        );
        let player = desktop_entry(directory.path(), "player", "Player", "/bin/true", "player");
        let source = IconSource {
            binary: Some("true".into()),
            name: Some("Player".into()),
            ..Default::default()
        };
        for apps in [
            [work.clone(), player.clone()],
            [player.clone(), work.clone()],
        ] {
            assert_eq!(desktop_icon(&source, &apps), player.icon());
            for name in [None, Some("Unknown"), Some("true")] {
                assert!(desktop_icon(
                    &IconSource {
                        name: name.map(Into::into),
                        ..source.clone()
                    },
                    &apps
                )
                .is_none());
            }
            assert_eq!(
                desktop_icon(
                    &IconSource {
                        application_id: Some("aaa-work.desktop".into()),
                        ..source.clone()
                    },
                    &apps
                ),
                work.icon()
            );
        }
    }

    #[test]
    fn ambiguous_names_and_executable_aliases_keep_the_neutral_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let first = desktop_entry(directory.path(), "first", "Player", "/bin/true", "first");
        let second = desktop_entry(directory.path(), "second", "Player", "/bin/false", "second");
        let variant = desktop_entry(
            directory.path(),
            "variant",
            "Player Beta",
            "/bin/true",
            "beta",
        );
        let apps = [first.clone(), second, variant];
        let source = IconSource {
            name: Some("Player".into()),
            ..Default::default()
        };
        assert!(desktop_icon(&source, &apps).is_none());
        assert_eq!(
            desktop_icon(
                &IconSource {
                    binary: Some("false".into()),
                    ..source.clone()
                },
                &apps
            ),
            apps[1].icon()
        );
        assert_eq!(
            desktop_icon(
                &IconSource {
                    binary: Some("true".into()),
                    ..source
                },
                &apps
            ),
            first.icon()
        );
        assert!(desktop_icon(
            &IconSource {
                name: Some("true".into()),
                ..Default::default()
            },
            &apps
        )
        .is_none());
        let duplicate = desktop_entry(
            directory.path(),
            "duplicate",
            "Player",
            "/bin/true",
            "duplicate",
        );
        assert!(desktop_icon(
            &IconSource {
                binary: Some("true".into()),
                name: Some("Player".into()),
                ..Default::default()
            },
            &[first, duplicate]
        )
        .is_none());
    }

    #[test]
    fn native_and_flatpak_commands_use_the_same_ambiguity_rules() {
        let directory = tempfile::tempdir().unwrap();
        let native = desktop_entry(
            directory.path(),
            "native",
            "Native Player",
            "/bin/true",
            "native",
        );
        let sandbox = flatpak_desktop(
            directory.path(),
            "org.example.Player",
            "--command=true",
            "sandbox",
        );
        for apps in [
            [native.clone(), sandbox.clone()],
            [sandbox.clone(), native.clone()],
        ] {
            let source = IconSource {
                binary: Some("true".into()),
                ..Default::default()
            };
            assert!(desktop_icon(&source, &apps).is_none());
            assert_eq!(
                desktop_icon(
                    &IconSource {
                        name: Some("腾讯会议".into()),
                        ..source
                    },
                    &apps
                ),
                sandbox.icon()
            );
        }
    }

    #[test]
    fn missing_artwork_does_not_select_a_weaker_or_ambiguous_launcher() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.desktop");
        std::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Player\nExec=/bin/true\n",
        )
        .unwrap();
        let missing: gio::AppInfo = gio::DesktopAppInfo::from_filename(path).unwrap().upcast();
        assert!(missing.icon().is_none());
        let other = desktop_entry(directory.path(), "other", "Other", "/bin/false", "other");
        let apps = [missing.clone(), other];
        for source in [
            IconSource {
                application_id: Some("missing".into()),
                binary: Some("false".into()),
                name: Some("Other".into()),
                ..Default::default()
            },
            IconSource {
                binary: Some("true".into()),
                name: Some("Other".into()),
                ..Default::default()
            },
        ] {
            assert!(desktop_icon(&source, &apps).is_none());
        }
        let variant = desktop_entry(
            directory.path(),
            "variant",
            "Player Beta",
            "/bin/false",
            "beta",
        );
        assert!(desktop_icon(
            &IconSource {
                name: Some("Player".into()),
                ..Default::default()
            },
            &[missing.clone(), variant]
        )
        .is_none());
        let other = desktop_entry(directory.path(), "shared", "Other", "/bin/true", "other");
        assert!(desktop_icon(
            &IconSource {
                binary: Some("true".into()),
                ..Default::default()
            },
            &[missing, other]
        )
        .is_none());
    }

    #[test]
    fn flatpak_audio_binaries_match_unique_exported_commands_without_pid_or_name_hints() {
        let directory = tempfile::tempdir().unwrap();
        let mut properties = libpulse_binding::proplist::Proplist::new().unwrap();
        for (key, value) in [
            ("application.name", "Wemeet VoiceEngine"),
            ("application.process.binary", "wemeetapp"),
            ("application.process.id", "2"),
        ] {
            properties.set_str(key, value).unwrap();
        }
        let source = IconSource::from_properties(&properties);
        for options in [
            "--branch=stable --command=wemeetapp --file-forwarding",
            "--command wemeetapp",
            "--command=\"/app/Meeting Files/wemeetapp\"",
        ] {
            let app = flatpak_desktop(directory.path(), "com.example.Meeting", options, "meeting");
            assert_eq!(
                desktop_icon(&source, std::slice::from_ref(&app)),
                app.icon()
            );
        }
        for options in [
            "--command=wemeetapp-helper",
            "--command=unrelated",
            "wemeetapp",
        ] {
            let app = flatpak_desktop(directory.path(), "com.example.Meeting", options, "meeting");
            assert!(desktop_icon(&source, &[app]).is_none());
        }
        let primary = flatpak_desktop(
            directory.path(),
            "com.example.Meeting",
            "--command=wemeetapp",
            "meeting",
        );
        let beta = flatpak_desktop(
            directory.path(),
            "com.example.Meeting.Beta",
            "--command=wemeetapp",
            "meeting-beta",
        );
        let apps = [primary, beta];
        assert!(desktop_icon(&source, &apps).is_none());
        assert_eq!(
            desktop_icon(
                &IconSource {
                    application_id: Some("com.example.Meeting.Beta".into()),
                    ..source
                },
                &apps
            ),
            apps[1].icon()
        );
    }

    #[test]
    fn flatpak_exported_suffixes_respect_application_id_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let id = "com.example.Meeting";
        let source = IconSource {
            binary: Some("wemeetapp".into()),
            ..Default::default()
        };
        for (suffix, allowed) in [
            ("", true),
            (".viewer", true),
            ("-viewer", true),
            ("Extra", false),
            (".", false),
            ("-", false),
        ] {
            let app = flatpak_desktop_with_id(
                directory.path(),
                &format!("{id}{suffix}"),
                id,
                "--command=wemeetapp",
                "meeting",
            );
            assert_eq!(
                desktop_icon(&source, std::slice::from_ref(&app)).is_some(),
                allowed,
                "desktop suffix: {suffix}"
            );
        }
    }

    #[test]
    fn browser_variant_names_require_a_unique_candidate_after_exact_matches() {
        let desktop = |name: &str, icon: &str| {
            let keyfile = gtk::glib::KeyFile::new();
            keyfile.load_from_data(&format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\nIcon={icon}\n"), gtk::glib::KeyFileFlags::NONE).unwrap();
            gio::DesktopAppInfo::from_keyfile(&keyfile)
                .unwrap()
                .upcast::<gio::AppInfo>()
        };
        let origin = desktop("Brave Origin", "brave-origin");
        let beta = desktop("Brave-Beta", "brave-beta");
        let exact = desktop("Brave", "brave-browser");
        let mut properties = libpulse_binding::proplist::Proplist::new().unwrap();
        for (key, value) in [
            ("application.name", "Brave"),
            ("application.process.binary", "brave"),
            ("application.icon_name", "brave-browser"),
        ] {
            properties.set_str(key, value).unwrap();
        }
        let source = IconSource::from_properties(&properties);
        assert_eq!(
            desktop_icon(&source, std::slice::from_ref(&origin)),
            origin.icon()
        );
        assert!(desktop_icon(&source, &[origin.clone(), beta.clone()]).is_none());
        assert_eq!(
            desktop_icon(&source, &[origin, beta, exact.clone()]),
            exact.icon()
        );
        for candidate in ["Bravely", "Other Brave", "Brave ", "Brave-", "Brave"] {
            assert!(!variant_name(candidate, "Brave"));
        }
        assert!(variant_name("brave origin", "Brave"));
    }

    #[test]
    #[ignore = "native GTK icon loading; requires a display or xvfb-run"]
    fn native_theme_and_desktop_icons_are_bounded_pngs_with_safe_fallbacks() {
        use base64::Engine as _;
        gtk::init().expect("the native icon regression requires GTK and a display");
        let directory = tempfile::tempdir().unwrap();
        let theme_dir = directory.path().join("mimi-fixture");
        let icon_dir = theme_dir.join("32x32/apps");
        std::fs::create_dir_all(&icon_dir).unwrap();
        std::fs::write(theme_dir.join("index.theme"), "[Icon Theme]\nName=Mimi fixture\nDirectories=32x32/apps\n[32x32/apps]\nSize=32\nType=Fixed\nContext=Applications\n").unwrap();
        std::fs::write(
            icon_dir.join("fixture-player.png"),
            include_bytes!("../../icons/32x32.png"),
        )
        .unwrap();
        std::fs::write(icon_dir.join("fixture-vector.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"><rect width=\"64\" height=\"64\" fill=\"#129abc\"/></svg>").unwrap();
        std::fs::write(icon_dir.join("broken.png"), b"invalid image").unwrap();
        let oversized = std::fs::File::create(icon_dir.join("oversized.png")).unwrap();
        oversized.set_len(MAX_ICON_FILE_BYTES + 1).unwrap();
        let theme = IconTheme::new();
        theme.prepend_search_path(directory.path());
        theme.set_custom_theme(Some("mimi-fixture"));
        let desktop = |id: &str, name: &str, binary: &str, icon: &str| {
            let path = directory.path().join(format!("{id}.desktop"));
            std::fs::write(
                &path,
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nExec={binary}\nIcon={icon}\n"
                ),
            )
            .unwrap();
            gio::DesktopAppInfo::from_filename(path)
                .unwrap()
                .upcast::<gio::AppInfo>()
        };
        let player = desktop(
            "org.example.Player",
            "Native Player",
            "/bin/true",
            "fixture-player",
        );
        let vector = desktop(
            "org.example.Vector",
            "Vector Player",
            "/bin/false",
            "fixture-vector",
        );
        let absolute = desktop(
            "org.example.Absolute",
            "Absolute Player",
            "/bin/true",
            icon_dir.join("fixture-vector.svg").to_str().unwrap(),
        );
        let browser = desktop(
            "brave-origin",
            "Brave Origin",
            "/bin/true",
            "fixture-vector",
        );
        let meeting = flatpak_desktop(
            directory.path(),
            "com.example.Meeting",
            "--branch=stable --command=wemeetapp --file-forwarding",
            "fixture-vector",
        );
        let installed = vec![vector, player, absolute, browser, meeting];
        let sources = vec![
            Some(IconSource {
                icon_names: vec!["fixture-player".into()],
                ..Default::default()
            }),
            Some(IconSource {
                application_id: Some("org.example.Player.desktop".into()),
                binary: Some("false".into()),
                ..Default::default()
            }),
            Some(IconSource {
                binary: Some("false".into()),
                ..Default::default()
            }),
            Some(IconSource {
                name: Some("Native Player".into()),
                ..Default::default()
            }),
            Some(IconSource {
                application_id: Some("org.example.Absolute".into()),
                ..Default::default()
            }),
            Some(IconSource {
                icon_names: vec!["missing".into(), "broken".into()],
                name: Some("Native Player".into()),
                ..Default::default()
            }),
            Some(IconSource {
                icon_names: vec!["missing".into(), "broken".into(), "oversized".into()],
                ..Default::default()
            }),
            None,
            Some(IconSource {
                icon_names: vec!["mimi-fixture-unavailable-browser".into()],
                binary: Some("brave".into()),
                name: Some("Brave".into()),
                ..Default::default()
            }),
            Some(IconSource {
                binary: Some("wemeetapp".into()),
                name: Some("Wemeet VoiceEngine".into()),
                ..Default::default()
            }),
        ];
        let icons = encode_sources(&theme, &sources, || installed);
        assert!(icons[..6].iter().all(Option::is_some));
        assert_eq!(icons[0], icons[1]); // Desktop ID wins over a conflicting binary.
        assert_eq!(icons[0], icons[3]);
        assert_eq!(icons[0], icons[5]); // Broken explicit artwork falls back locally.
        assert_ne!(icons[0], icons[2]);
        assert_eq!(icons[2], icons[4]); // Themed and local absolute SVG both render.
        assert!(icons[6..8].iter().all(Option::is_none));
        assert_eq!(icons[2], icons[8]); // A unique launcher variant supplies its own icon.
        assert_eq!(icons[2], icons[9]); // Flatpak's command resolves the audio binary.
        for icon in icons.into_iter().flatten() {
            let png = base64::engine::general_purpose::STANDARD
                .decode(icon.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            assert!(png.len() <= super::super::applications::MAX_APPLICATION_ICON_PNG_BYTES);
            let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(png)).unwrap();
            assert_eq!((pixbuf.width(), pixbuf.height()), (ICON_SIZE, ICON_SIZE));
        }
        let repeated = vec![sources[0].clone(); 512];
        let icons = encode_sources(&theme, &repeated, || {
            panic!("valid theme names must not enumerate desktop entries")
        });
        assert_eq!(icons.len(), 512);
        assert!(
            icons
                .into_iter()
                .flatten()
                .map(|icon| icon.len())
                .sum::<usize>()
                <= 1024 * 1024
        );
    }
}
