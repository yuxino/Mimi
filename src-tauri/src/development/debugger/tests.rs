use super::*;

#[test]
fn evidence_workspace_is_ignored_before_the_exact_dev_gate() {
    let root = tempfile::tempdir().unwrap();
    assert!(initialize_evidence_storage(
        false,
        || panic!("production must not resolve the evidence root"),
        || panic!("production must not read the workspace environment"),
    )
    .unwrap()
    .is_none());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn invalid_evidence_workspace_fails_before_creating_directories() {
    let root = tempfile::tempdir().unwrap();
    for selector in ["../outside", "bad/name", "-batch", "白"] {
        assert_eq!(
            initialize_evidence_storage(
                true,
                || Some(root.path().join("app")),
                || Some(selector.into()),
            ),
            Err("development_evidence_workspace_invalid")
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            initialize_evidence_storage(
                true,
                || Some(root.path().join("app")),
                || Some(std::ffi::OsString::from_vec(vec![0xff])),
            ),
            Err("development_evidence_workspace_invalid")
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn evidence_workspaces_preserve_the_default_and_limit_extra_catalogs() {
    let app = tempfile::tempdir().unwrap();
    let legacy = app.path().join("development-evidence");
    private_directory(&legacy).unwrap();
    fs::write(legacy.join("unchanged.json"), b"synthetic saved evidence").unwrap();
    for selector in [None, Some(""), Some("default")] {
        let (_, root, workspace) = initialize_evidence_storage(
            true,
            || Some(app.path().to_owned()),
            || selector.map(Into::into),
        )
        .unwrap()
        .unwrap();
        assert_eq!(root, legacy);
        assert_eq!(workspace.label(), "default");
    }
    let mut roots = Vec::new();
    for index in 0..EXTRA_WORKSPACE_LIMIT {
        let workspace = EvidenceWorkspace::parse(Some(&format!("batch-{index}"))).unwrap();
        roots.push(select_evidence_root(app.path(), &workspace).unwrap());
    }
    let ninth = EvidenceWorkspace::parse(Some("batch-9")).unwrap();
    assert_eq!(
        select_evidence_root(app.path(), &ninth),
        Err("development_evidence_workspace_limit")
    );
    assert!(!workspace_directory(app.path(), &ninth).exists());
    for (index, root) in roots.iter().enumerate() {
        let workspace = EvidenceWorkspace::parse(Some(&format!("batch-{index}"))).unwrap();
        assert_eq!(select_evidence_root(app.path(), &workspace).unwrap(), *root);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
    assert_eq!(
        select_evidence_root(app.path(), &EvidenceWorkspace::Default).unwrap(),
        legacy
    );
    assert_eq!(
        fs::read(legacy.join("unchanged.json")).unwrap(),
        b"synthetic saved evidence"
    );
    assert_eq!(
        workspace_count(&app.path().join("development-evidence-workspaces")).unwrap(),
        EXTRA_WORKSPACE_LIMIT
    );
    assert_eq!(
        ALL_CASES_BYTE_LIMIT * (EXTRA_WORKSPACE_LIMIT as u64 + 1),
        1152 * 1024 * 1024
    );
}

#[test]
fn evidence_workspace_ignores_only_bounded_regular_finder_metadata() {
    let container = tempfile::tempdir().unwrap();
    let metadata = container.path().join(".DS_Store");
    let original = vec![0x42; 64 * 1024];
    fs::write(&metadata, &original).unwrap();
    for index in 0..EXTRA_WORKSPACE_LIMIT {
        fs::create_dir(container.path().join(format!("batch-{index}"))).unwrap();
    }
    let evidence = container.path().join("batch-0").join("unchanged.json");
    fs::write(&evidence, b"synthetic immutable case").unwrap();
    assert_eq!(
        workspace_count(container.path()).unwrap(),
        EXTRA_WORKSPACE_LIMIT
    );
    assert_eq!(fs::read(&metadata).unwrap(), original);
    assert_eq!(fs::read(&evidence).unwrap(), b"synthetic immutable case");
    fs::create_dir(container.path().join("ninth")).unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_limit")
    );
}

#[test]
fn evidence_workspace_rejects_oversized_metadata_and_other_entries() {
    let container = tempfile::tempdir().unwrap();
    let metadata = container.path().join(".DS_Store");
    private_file(&metadata)
        .unwrap()
        .set_len(64 * 1024 + 1)
        .unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_invalid")
    );
    assert_eq!(fs::metadata(&metadata).unwrap().len(), 64 * 1024 + 1);
    fs::remove_file(&metadata).unwrap();
    fs::create_dir(&metadata).unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_invalid")
    );
    fs::remove_dir(&metadata).unwrap();
    fs::write(
        container.path().join("other-metadata"),
        b"bounded but unrecognized",
    )
    .unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_invalid")
    );
}

#[cfg(unix)]
#[test]
fn evidence_workspace_rejects_finder_metadata_symlinks() {
    use std::os::unix::fs::symlink;
    let container = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("metadata");
    fs::write(&target, b"unchanged outside fixture").unwrap();
    let link = container.path().join(".DS_Store");
    symlink(&target, &link).unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_invalid")
    );
    assert_eq!(fs::read(&target).unwrap(), b"unchanged outside fixture");
    fs::remove_file(&link).unwrap();
    symlink(outside.path(), &link).unwrap();
    assert_eq!(
        workspace_count(container.path()),
        Err("development_evidence_workspace_invalid")
    );
}

#[test]
fn evidence_workspace_retains_the_same_per_catalog_case_reservation() {
    let app = tempfile::tempdir().unwrap();
    let legacy = select_evidence_root(app.path(), &EvidenceWorkspace::Default).unwrap();
    private_file(&legacy.join("existing.bin"))
        .unwrap()
        .set_len(ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1)
        .unwrap();
    assert!(matches!(
        start_case(legacy.clone(), &serde_json::json!({}), Instant::now()),
        Err("development_evidence_storage_limit")
    ));
    let workspace = EvidenceWorkspace::parse(Some("second-batch")).unwrap();
    let root = select_evidence_root(app.path(), &workspace).unwrap();
    let case = start_case(root.clone(), &serde_json::json!({}), Instant::now()).unwrap();
    case.tx.lock().unwrap().take();
    case.writer.lock().unwrap().take().unwrap().join().unwrap();
    assert!(case.directory.starts_with(&root));
    assert_eq!(
        stored_bytes(&legacy).unwrap(),
        ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1
    );
    for index in 1..CASE_COUNT_LIMIT {
        fs::create_dir(root.join(format!("occupied-{index}"))).unwrap();
    }
    assert!(matches!(
        start_case(root.clone(), &serde_json::json!({}), Instant::now()),
        Err("development_evidence_storage_limit")
    ));
    assert_eq!(fs::read_dir(root).unwrap().count(), CASE_COUNT_LIMIT);
}

#[test]
fn evidence_workspace_rejects_non_directories_and_changed_roots() {
    let app = tempfile::tempdir().unwrap();
    let workspace = EvidenceWorkspace::parse(Some("batch")).unwrap();
    let root = select_evidence_root(app.path(), &workspace).unwrap();
    let debugger = DebuggerState {
        app_directory: Some(app.path().to_owned()),
        root: Some(root.clone()),
        workspace,
        ..Default::default()
    };
    assert_eq!(evidence_root(&debugger).unwrap(), root);
    fs::remove_dir(&root).unwrap();
    fs::write(&root, b"not a directory").unwrap();
    assert_eq!(
        evidence_root(&debugger),
        Err("development_evidence_workspace_invalid")
    );
    assert_eq!(
        select_evidence_root(app.path(), &debugger.workspace),
        Err("development_evidence_workspace_invalid")
    );
    assert_eq!(fs::read(root).unwrap(), b"not a directory");
}

#[cfg(unix)]
#[test]
fn evidence_workspace_rejects_symlink_roots_without_touching_targets() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(target.join("unchanged"), b"outside fixture").unwrap();
    let linked_app = root.path().join("linked-app");
    symlink(&target, &linked_app).unwrap();
    assert_eq!(
        select_evidence_root(&linked_app, &EvidenceWorkspace::Default),
        Err("development_evidence_workspace_invalid")
    );
    for (name, workspace) in [
        ("development-evidence", EvidenceWorkspace::Default),
        (
            "development-evidence-workspaces",
            EvidenceWorkspace::parse(Some("batch")).unwrap(),
        ),
        (
            "development-evidence-workspaces/batch",
            EvidenceWorkspace::parse(Some("batch")).unwrap(),
        ),
    ] {
        let app = tempfile::tempdir().unwrap();
        let link = app.path().join(name);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&target, &link).unwrap();
        assert_eq!(
            select_evidence_root(app.path(), &workspace),
            Err("development_evidence_workspace_invalid")
        );
    }
    assert_eq!(
        fs::read(target.join("unchanged")).unwrap(),
        b"outside fixture"
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

fn metadata_entry(id: u64) -> trace::DebugEntry {
    trace::DebugEntry {
        id,
        elapsed_ms: id,
        event: trace::DebugEvent::Pipeline {
            label: "capture started".into(),
        },
    }
}
#[test]
fn persisted_metadata_exceeds_the_live_ring_and_pages_exports_every_id() {
    let root = tempfile::tempdir().unwrap();
    let case = start_case(
        root.path().to_owned(),
        &serde_json::json!({}),
        Instant::now(),
    )
    .unwrap();
    for id in 1..=(trace::ENTRY_LIMIT + 1) as u64 {
        let mut entry = metadata_entry(id);
        if id == (trace::ENTRY_LIMIT + 1) as u64 {
            entry.event = trace::DebugEvent::Stopped {
                unflushed_windows: vec![],
            };
        }
        let mut bytes = serde_json::to_vec(&entry).unwrap();
        bytes.push(b'\n');
        case.tx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send((ContentKind::Trace, bytes))
            .unwrap();
    }
    seal_case_trace(&case, std::time::Duration::ZERO).unwrap();
    case.writer.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(
        case.trace_entries.load(Ordering::Relaxed),
        (trace::ENTRY_LIMIT + 1) as u64
    );
    assert_eq!(case.trace_dropped.load(Ordering::Relaxed), 0);
    let first = read_trace_page(&case.directory, 0, 64).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(first[0].id, 1);
    let last = read_trace_page(&case.directory, trace::ENTRY_LIMIT, 64).unwrap();
    assert_eq!(last.len(), 1);
    assert!(matches!(last[0].event, trace::DebugEvent::Stopped { .. }));
    let destination = root.path().join("export.jsonl");
    assert!(copy_case_file(
        &case.directory,
        "trace-events.jsonl",
        &destination,
        CONTENT_BYTE_LIMIT
    )
    .unwrap());
    let stored = fs::read(case.directory.join("trace-events.jsonl")).unwrap();
    assert_eq!(fs::read(destination).unwrap(), stored);
    assert_eq!(
        case.trace_bytes.load(Ordering::Relaxed),
        stored.len() as u64
    );
    assert_eq!(case.bytes.load(Ordering::Relaxed), stored.len() as u64);
    assert!(read_trace_page(&case.directory, 0, 65).is_err());
    assert!(read_trace_page(&case.directory, 0, 0).is_err());
    assert!(!copy_case_file(
        root.path(),
        "missing-old-case-trace.jsonl",
        &root.path().join("unused.jsonl"),
        CONTENT_BYTE_LIMIT
    )
    .unwrap());
}
#[test]
fn durable_metadata_reports_queue_line_and_shared_disk_budget_loss() {
    let root = tempfile::tempdir().unwrap();
    let case = start_case(
        root.path().to_owned(),
        &serde_json::json!({}),
        Instant::now(),
    )
    .unwrap();
    let gate = case.write_gate.lock().unwrap();
    for id in 1..=80 {
        record_trace_event(&case, metadata_entry(id));
    }
    assert!(case.trace_dropped.load(Ordering::Relaxed) >= 47);
    drop(gate);
    seal_case_trace(&case, std::time::Duration::ZERO).unwrap();
    case.writer.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(
        case.trace_entries.load(Ordering::Relaxed) + case.trace_dropped.load(Ordering::Relaxed),
        80
    );
    assert_eq!(
        case.trace_dropped.load(Ordering::Relaxed),
        case.dropped.load(Ordering::Relaxed)
    );

    let limited = start_case(
        root.path().to_owned(),
        &serde_json::json!({}),
        Instant::now(),
    )
    .unwrap();
    let mut huge = metadata_entry(1);
    huge.event = trace::DebugEvent::Pipeline {
        label: "x".repeat(512 * 1024),
    };
    record_trace_event(&limited, huge);
    assert_eq!(limited.trace_dropped.load(Ordering::Relaxed), 1);
    limited.bytes.store(CONTENT_BYTE_LIMIT, Ordering::Relaxed);
    record_trace_event(&limited, metadata_entry(2));
    seal_case_trace(&limited, std::time::Duration::ZERO).unwrap();
    limited
        .writer
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .join()
        .unwrap();
    assert!(limited.trace_limited.load(Ordering::Relaxed));
    assert!(limited.limited.load(Ordering::Relaxed));
    assert_eq!(limited.trace_entries.load(Ordering::Relaxed), 0);
    assert_eq!(limited.trace_dropped.load(Ordering::Relaxed), 2);
}
#[test]
fn timeout_keeps_the_case_unsealed_and_retry_drains_the_stopped_entry() {
    let root = tempfile::tempdir().unwrap();
    let case = start_case(
        root.path().to_owned(),
        &serde_json::json!({}),
        Instant::now(),
    )
    .unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let target_case = case.clone();
    let target = trace::TraceSink::new(move |entry| {
        ready_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv().unwrap();
        record_trace_event(&target_case, entry);
    });
    *case.trace_sink.lock().unwrap() = Some(target.clone());
    let worker = std::thread::spawn(move || {
        target.dispatch_for_test(trace::DebugEntry {
            id: 1,
            elapsed_ms: 0,
            event: trace::DebugEvent::Stopped {
                unflushed_windows: vec![],
            },
        })
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        seal_case_trace(&case, std::time::Duration::from_millis(1)),
        Err("development_evidence_trace_busy")
    );
    assert!(case.tx.lock().unwrap().is_some());
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    seal_case_trace(&case, std::time::Duration::from_secs(1)).unwrap();
    case.writer.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(case.trace_entries.load(Ordering::Relaxed), 1);
    assert_eq!(case.trace_dropped.load(Ordering::Relaxed), 0);
    assert!(matches!(
        read_trace_page(&case.directory, 0, 64).unwrap()[0].event,
        trace::DebugEvent::Stopped { .. }
    ));
}
#[cfg(unix)]
#[test]
fn trace_reads_and_export_refuse_external_symlinks_and_nonregular_files() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("private.txt"), b"private marker").unwrap();
    symlink(
        outside.path().join("private.txt"),
        root.path().join("trace-events.jsonl"),
    )
    .unwrap();
    assert!(read_trace_page(root.path(), 0, 64).is_err());
    let destination = root.path().join("export.jsonl");
    assert!(copy_case_file(
        root.path(),
        "trace-events.jsonl",
        &destination,
        CONTENT_BYTE_LIMIT
    )
    .is_err());
    assert!(!destination.exists());
    fs::remove_file(root.path().join("trace-events.jsonl")).unwrap();
    fs::create_dir(root.path().join("trace-events.jsonl")).unwrap();
    assert!(read_trace_page(root.path(), 0, 64).is_err());
}

#[tokio::test]
async fn operation_lease_excludes_other_operations_across_await_and_recovers_on_drop() {
    let gate = OperationGate::default();
    let operation = gate.try_begin().unwrap();
    tokio::task::yield_now().await;
    assert!(matches!(gate.try_begin(), Err("development_debug_busy")));
    drop(operation);
    let operation = gate.try_begin().unwrap();
    drop(operation);
    assert!(gate.try_begin().is_ok());
}

#[test]
fn private_case_seals_and_reopens_the_same_bounded_recording() {
    let root = tempfile::tempdir().unwrap();
    let route = serde_json::json!({"provider":"alibabaCloud","createdAtUnixMs":123});
    let case = start_case(root.path().to_owned(), &route, Instant::now()).unwrap();
    record_private_event(
        &case,
        serde_json::json!({"kind":"synthetic","source":"example"}),
    );
    case.tx
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .send((
            ContentKind::Snapshot,
            b"{\"snapshot\":{\"synthetic\":true}}\n".to_vec(),
        ))
        .unwrap();
    case.tx.lock().unwrap().take();
    case.writer.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(case.private_events.load(Ordering::Relaxed), 1);
    assert_eq!(case.snapshots.load(Ordering::Relaxed), 1);
    let event_bytes = fs::read(case.directory.join("events.jsonl")).unwrap();
    let snapshot_bytes = fs::read(case.directory.join("snapshots.jsonl")).unwrap();
    assert_eq!(
        case.bytes.load(Ordering::Relaxed),
        (event_bytes.len() + snapshot_bytes.len()) as u64
    );
    // A late callback to a sealed case cannot append or enter a new case.
    record_private_event(&case, serde_json::json!({"kind":"late"}));
    assert_eq!(
        fs::read(case.directory.join("events.jsonl")).unwrap(),
        event_bytes
    );
    let mut trace = trace::snapshot(0);
    trace.enabled = false;
    let mut report = DebuggerSnapshot {
        trace,
        route,
        audio: serde_json::json!({}),
        case_id: Some(case.id.clone()),
        replay_snapshots: 1,
        private_events: 1,
        trace_persistence_enabled: true,
        persisted_trace_entries: 0,
        trace_bytes: 0,
        trace_dropped: 0,
        trace_limited: false,
        trace_failed: false,
        content_bytes: case.bytes.load(Ordering::Relaxed),
        content_dropped: 0,
        content_limited: false,
        content_failed: false,
    };
    persist_case_report(&case.directory, &report).unwrap();
    let reopened =
        read_saved_report(&saved_case_directory(root.path(), &case.id).unwrap()).unwrap();
    assert_eq!(reopened.case_id, Some(case.id.clone()));
    assert_eq!(reopened.private_events, 1);
    assert_eq!(reopened.replay_snapshots, 1);
    assert_eq!(reopened.route["evidenceWorkspace"], "default");
    let mut legacy = serde_json::to_value(&report).unwrap();
    for name in [
        "tracePersistenceEnabled",
        "persistedTraceEntries",
        "traceBytes",
        "traceDropped",
        "traceLimited",
        "traceFailed",
    ] {
        legacy.as_object_mut().unwrap().remove(name);
    }
    let legacy: DebuggerSnapshot = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.trace_persistence_enabled);
    assert_eq!(legacy.persisted_trace_entries, 0);
    assert!(saved_case_directory(root.path(), "../other").is_err());
    report.trace.enabled = true;
    persist_case_report(&case.directory, &report).unwrap();
    assert!(matches!(
        read_saved_report(&case.directory),
        Err("development_case_invalid")
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&case.directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for name in [
            "manifest.json",
            "events.jsonl",
            "snapshots.jsonl",
            "trace.json",
        ] {
            assert_eq!(
                fs::metadata(case.directory.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn starting_a_case_reserves_the_entire_case_budget() {
    let root = tempfile::tempdir().unwrap();
    private_file(&root.path().join("existing.bin"))
        .unwrap()
        .set_len(ALL_CASES_BYTE_LIMIT - CASE_RESERVED_BYTES + 1)
        .unwrap();
    assert!(matches!(
        start_case(
            root.path().to_owned(),
            &serde_json::json!({}),
            Instant::now()
        ),
        Err("development_evidence_storage_limit")
    ));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn case_bound_audio_and_export_do_not_read_another_cases_recording() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    for (directory, sample) in [(&first, 0x11), (&second, 0x22)] {
        private_directory(&directory.join("audio-evidence")).unwrap();
        let mut wav = vec![0; 48];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        wav[44..48].fill(sample);
        private_file(&case_audio_path(directory, AudioSource::System))
            .unwrap()
            .write_all(&wav)
            .unwrap();
    }
    let captured = first.clone();
    assert!(validate_case_id("second", "first").is_err());
    assert_eq!(
        &read_case_audio(&captured, AudioSource::System).unwrap()[44..],
        &[0x11; 4]
    );
    let destination = root.path().join("export.wav");
    assert!(copy_case_audio(&captured, AudioSource::System, &destination).unwrap());
    assert_eq!(&fs::read(&destination).unwrap()[44..], &[0x11; 4]);
    assert!(!copy_case_audio(
        &captured,
        AudioSource::Microphone,
        &root.path().join("missing.wav")
    )
    .unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(destination).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let oversize = private_file(&case_audio_path(&first, AudioSource::Microphone)).unwrap();
    oversize.set_len(AUDIO_FILE_BYTE_LIMIT + 1).unwrap();
    assert!(read_case_audio(&first, AudioSource::Microphone).is_err());
    assert!(copy_case_audio(
        &first,
        AudioSource::Microphone,
        &root.path().join("oversize.wav")
    )
    .is_err());
}
