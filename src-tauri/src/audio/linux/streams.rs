//! Metadata only: one worker owns every libpulse object. Tokens identify a
//! connection and a stream lifetime, never an application or a reusable index.
use super::applications::{ApplicationSnapshot, AudioApplication};
use super::linux::{check_context, wait_for, ConnectedContext, WorkerControl, START_TIMEOUT};
use super::SystemAudioCaptureError as Error;
use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::subscribe::{Facility, InterestMaskSet, Operation};
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const LIMIT: usize = 512;
const IDLE_POLL: Duration = Duration::from_millis(250);

#[derive(Clone)]
struct Metadata {
    index: u32,
    sink: u32,
    stream: Option<String>,
    icon_source: super::linux_application_icons::IconSource,
}

#[derive(Default)]
struct Registry {
    entries: BTreeMap<u32, (String, Metadata)>,
    revision: Arc<AtomicU64>,
    connected: bool,
}

impl Registry {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    fn invalidate(&mut self) {
        self.entries.clear();
        self.connected = false;
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn event(&mut self, index: u32, operation: Operation) {
        if operation != Operation::Changed {
            self.entries.remove(&index);
        }
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn reconcile(&mut self, revision: u64, metadata: BTreeMap<u32, Metadata>) -> bool {
        // A delete/new/change observed during a query supersedes every result.
        // In particular a late list item must not resurrect a retired token.
        if !self.connected || self.revision() != revision {
            return false;
        }
        self.entries.retain(|index, _| metadata.contains_key(index));
        for (index, item) in metadata.into_iter().take(LIMIT) {
            if let Some((_, old)) = self.entries.get_mut(&index) {
                *old = item;
            } else {
                self.entries.insert(
                    index,
                    (format!("linux-stream:{}", uuid::Uuid::new_v4()), item),
                );
            }
        }
        // Publish completed refreshes too: a Changed event can be observed
        // before its new sink metadata is available to capture.
        self.revision.fetch_add(1, Ordering::Release);
        true
    }

    fn resolve(&self, token: &str) -> Result<(u32, u32), Error> {
        if !self.connected {
            return Err(Error::ApplicationUnavailable);
        }
        self.entries
            .values()
            .find_map(|(id, item)| (id == token).then_some((item.index, item.sink)))
            .ok_or(Error::ApplicationUnavailable)
    }

    fn snapshot(&self, language: &str) -> ApplicationSnapshot {
        let (unknown_app, unknown_stream) = placeholders(language);
        let names: Vec<_> = self
            .entries
            .values()
            .map(|(_, item)| {
                format!(
                    "{} · {}",
                    item.icon_source.name.as_deref().unwrap_or(unknown_app),
                    item.stream.as_deref().unwrap_or(unknown_stream)
                )
            })
            .collect();
        let mut counts = HashMap::with_capacity(names.len());
        for name in &names {
            *counts.entry(name.as_str()).or_insert(0usize) += 1;
        }
        let mut applications: Vec<_> = self
            .entries
            .values()
            .zip(&names)
            .map(|((id, item), name)| AudioApplication {
                id: id.clone(),
                name: if counts[name.as_str()] > 1 {
                    format!("{name} · #{}", item.index)
                } else {
                    name.clone()
                },
                icon_data_url: None,
            })
            .collect();
        super::applications::sort_applications(&mut applications);
        ApplicationSnapshot {
            supported: true,
            applications,
        }
    }
}

fn placeholders(language: &str) -> (&'static str, &'static str) {
    let language = crate::effective_native_menu_language(
        Some(language),
        crate::system_language_code().as_deref(),
    );
    match language {
        crate::NativeMenuLanguage::Chinese => ("未知应用", "未命名音频流"),
        crate::NativeMenuLanguage::TraditionalChinese => ("未知應用程式", "未命名音訊串流"),
        crate::NativeMenuLanguage::Japanese => ("不明なアプリ", "名前のない音声ストリーム"),
        crate::NativeMenuLanguage::Korean => ("알 수 없는 앱", "이름 없는 오디오 스트림"),
        crate::NativeMenuLanguage::German => ("Unbekannte Anwendung", "Unbenannter Audiostream"),
        crate::NativeMenuLanguage::French => ("Application inconnue", "Flux audio sans nom"),
        crate::NativeMenuLanguage::Thai => ("แอปที่ไม่รู้จัก", "สตรีมเสียงไม่มีชื่อ"),
        crate::NativeMenuLanguage::Russian => ("Неизвестное приложение", "Аудиопоток без названия"),
        _ => ("Unknown application", "Unnamed audio stream"),
    }
}

fn clean_name(value: Option<&str>) -> Option<String> {
    let mut bytes = 0;
    let mut value: String = value?
        .chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 100
        })
        .collect();
    value.truncate(value.trim_end().len());
    let start = value.len() - value.trim_start().len();
    value.drain(..start);
    (!value.is_empty()).then_some(value)
}

pub(super) fn application_name(properties: &pulse::proplist::Proplist) -> Option<String> {
    // Native PipeWire clients can expose their name only on the playback node.
    ["application.name", "node.name"]
        .into_iter()
        .find_map(|key| clean_name(properties.get_str(key).as_deref()))
}

struct Request {
    language: String,
    reply: oneshot::Sender<Result<ApplicationSnapshot, Error>>,
    #[cfg(test)]
    refresh_deadline: Option<Instant>,
}
struct Service {
    requests: mpsc::SyncSender<Request>,
    registry: Arc<Mutex<Registry>>,
    revision: Arc<AtomicU64>,
}
impl Service {
    fn resolve_if_changed(
        &self,
        token: &str,
        last_revision: &mut Option<u64>,
    ) -> Result<Option<(u32, u32)>, Error> {
        if *last_revision == Some(self.revision.load(Ordering::Acquire)) {
            return Ok(None);
        }
        let registry = self.registry.lock().unwrap();
        let target = registry.resolve(token)?;
        // Read while holding the lock so the cached version describes exactly
        // the metadata just validated, including concurrent refreshes.
        *last_revision = Some(registry.revision());
        Ok(Some(target))
    }
}
static SERVICE: OnceLock<Result<Service, ()>> = OnceLock::new();

/// A saved choice can be checked without initializing discovery or querying PulseAudio.
pub(super) fn cached(language: &str) -> ApplicationSnapshot {
    match SERVICE.get().and_then(|service| service.as_ref().ok()) {
        Some(service) => service.registry.lock().unwrap().snapshot(language),
        None => ApplicationSnapshot {
            supported: true,
            applications: Vec::new(),
        },
    }
}

/// Copy presentation hints for these exact live tokens, without discovery or
/// holding the registry lock during GTK/disk access. Retired tokens have none.
pub(super) fn icon_sources(
    applications: &[AudioApplication],
) -> Vec<Option<super::linux_application_icons::IconSource>> {
    let registry = SERVICE
        .get()
        .and_then(|service| service.as_ref().ok())
        .map(|service| service.registry.lock().unwrap());
    applications
        .iter()
        .map(|app| {
            registry
                .as_ref()?
                .entries
                .values()
                .find(|(id, _)| id == &app.id)
                .map(|(_, metadata)| metadata.icon_source.clone())
        })
        .collect()
}

pub async fn list(language: String) -> Result<ApplicationSnapshot, Error> {
    let service = SERVICE
        .get_or_init(|| {
            let (requests, receiver) = mpsc::sync_channel(16);
            let initial = Registry::default();
            let revision = Arc::clone(&initial.revision);
            let registry = Arc::new(Mutex::new(initial));
            let worker_registry = Arc::clone(&registry);
            std::thread::Builder::new()
                .name("mimi-playback-streams".into())
                .spawn(move || worker(receiver, worker_registry))
                .map_err(|_| ())?;
            Ok(Service {
                requests,
                registry,
                revision,
            })
        })
        .as_ref()
        .map_err(|_| Error::ApplicationListFailed)?;
    let (reply, response) = oneshot::channel();
    service
        .requests
        .try_send(Request {
            language,
            reply,
            #[cfg(test)]
            refresh_deadline: None,
        })
        .map_err(|_| Error::ApplicationListFailed)?;
    tokio::time::timeout(START_TIMEOUT + IDLE_POLL, response)
        .await
        .map_err(|_| Error::ApplicationListFailed)?
        .map_err(|_| Error::ApplicationListFailed)?
}

/// Inject a query timeout on the actual discovery worker, without a product IPC.
#[cfg(test)]
pub(super) async fn expired_refresh_for_test() -> Result<ApplicationSnapshot, Error> {
    let service = SERVICE.get().unwrap().as_ref().unwrap();
    let (reply, response) = oneshot::channel();
    service
        .requests
        .try_send(Request {
            language: "en".into(),
            reply,
            refresh_deadline: Some(Instant::now()),
        })
        .expect("the isolated test must enqueue its expired refresh");
    tokio::time::timeout(START_TIMEOUT + IDLE_POLL, response)
        .await
        .expect("the injected refresh must finish within the discovery timeout")
        .expect("the discovery worker must return the injected query result")
}

/// Does not initialize discovery: saved tokens cannot reopen capture after a restart.
pub(super) fn resolve(token: &str) -> Result<(u32, u32), Error> {
    SERVICE
        .get()
        .and_then(|service| service.as_ref().ok())
        .ok_or(Error::ApplicationUnavailable)?
        .registry
        .lock()
        .unwrap()
        .resolve(token)
}

/// Stable capture loops need only an atomic read, without locking discovery.
pub(super) fn resolve_if_changed(
    token: &str,
    last_revision: &mut Option<u64>,
) -> Result<Option<(u32, u32)>, Error> {
    SERVICE
        .get()
        .and_then(|service| service.as_ref().ok())
        .ok_or(Error::ApplicationUnavailable)?
        .resolve_if_changed(token, last_revision)
}

struct Connection {
    context: ConnectedContext,
    mainloop: Mainloop,
    dirty: Rc<Cell<bool>>,
}
impl Connection {
    fn connect(registry: &Arc<Mutex<Registry>>, deadline: Instant) -> Result<Self, Error> {
        let mut mainloop = Mainloop::new().ok_or(Error::AudioServerUnavailable)?;
        let mut context = ConnectedContext(
            Context::new(&mainloop, "mimi-stream-discovery")
                .ok_or(Error::AudioServerUnavailable)?,
        );
        context
            .connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|_| Error::AudioServerUnavailable)?;
        wait_for(
            &mut mainloop,
            &WorkerControl::default(),
            deadline,
            || match context.get_state() {
                State::Ready => Ok(Some(())),
                State::Failed | State::Terminated => Err(Error::AudioServerUnavailable),
                _ => Ok(None),
            },
        )?;
        registry.lock().unwrap().connected = true;
        let dirty = Rc::new(Cell::new(true));
        let changed = Rc::clone(&dirty);
        let events = Arc::clone(registry);
        context.set_subscribe_callback(Some(Box::new(move |facility, operation, index| {
            if facility == Some(Facility::SinkInput) {
                if let Some(operation) = operation {
                    events.lock().unwrap().event(index, operation);
                    changed.set(true);
                }
            }
        })));
        let subscribed = Rc::new(RefCell::new(None));
        let result = Rc::clone(&subscribed);
        let _subscription = context.subscribe(InterestMaskSet::SINK_INPUT, move |ok| {
            *result.borrow_mut() = Some(ok);
        });
        if !wait_for(&mut mainloop, &WorkerControl::default(), deadline, || {
            check_context(&context)?;
            Ok(subscribed.borrow_mut().take())
        })? {
            return Err(Error::ApplicationListFailed);
        }
        Ok(Self {
            context,
            mainloop,
            dirty,
        })
    }

    fn refresh(&mut self, registry: &Arc<Mutex<Registry>>, deadline: Instant) -> Result<(), Error> {
        loop {
            self.dirty.set(false);
            let revision = registry.lock().unwrap().revision();
            let items = Rc::new(RefCell::new(BTreeMap::new()));
            let results = Rc::clone(&items);
            let done = Rc::new(RefCell::new(None));
            let completed = Rc::clone(&done);
            let mut operation = self
                .context
                .introspect()
                .get_sink_input_info_list(move |result| match result {
                    ListResult::Item(info) => {
                        let mut items = results.borrow_mut();
                        if items.len() < LIMIT {
                            items.insert(
                                info.index,
                                Metadata {
                                    index: info.index,
                                    sink: info.sink,
                                    stream: clean_name(info.name.as_deref()),
                                    icon_source:
                                        super::linux_application_icons::IconSource::from_properties(
                                            &info.proplist,
                                        ),
                                },
                            );
                        }
                    }
                    ListResult::End => *completed.borrow_mut() = Some(true),
                    ListResult::Error => *completed.borrow_mut() = Some(false),
                });
            let result = wait_for(
                &mut self.mainloop,
                &WorkerControl::default(),
                deadline,
                || {
                    check_context(&self.context)?;
                    Ok(done.borrow_mut().take())
                },
            );
            if !matches!(result, Ok(true)) {
                operation.cancel();
                // Keep subscriptions and live identities after a query error.
                // The worker retries after its idle wait, rather than spinning.
                self.dirty.set(true);
                return Err(match result {
                    Err(Error::AudioServerUnavailable) => Error::AudioServerUnavailable,
                    _ => Error::ApplicationListFailed,
                });
            }
            if registry
                .lock()
                .unwrap()
                .reconcile(revision, std::mem::take(&mut *items.borrow_mut()))
            {
                return Ok(());
            }
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.context.set_subscribe_callback(None);
    }
}

fn refresh_connection(
    connection: &mut Option<Connection>,
    registry: &Arc<Mutex<Registry>>,
    deadline: Instant,
) -> Result<(), Error> {
    let result = (|| {
        if connection.is_none() {
            *connection = Some(Connection::connect(registry, deadline)?);
        }
        connection.as_mut().unwrap().refresh(registry, deadline)
    })();
    if result.is_err()
        && (matches!(result, Err(Error::AudioServerUnavailable))
            || connection
                .as_ref()
                .is_none_or(|current| current.context.get_state() != State::Ready))
    {
        registry.lock().unwrap().invalidate();
        *connection = None;
    }
    result
}

fn worker(requests: mpsc::Receiver<Request>, registry: Arc<Mutex<Registry>>) {
    let mut connection: Option<Connection> = None;
    loop {
        if let Some(current) = connection.as_mut() {
            let disconnected =
                !matches!(current.mainloop.iterate(false), IterateResult::Success(_))
                    || current.context.get_state() != State::Ready;
            if disconnected {
                registry.lock().unwrap().invalidate();
                connection = None;
            } else if current.dirty.get() {
                let _ =
                    refresh_connection(&mut connection, &registry, Instant::now() + START_TIMEOUT);
            }
        }
        // Requests wake this wait immediately. Only a connected server needs
        // periodic event dispatch; a disconnected worker has nothing to poll.
        let request = if connection.is_some() {
            requests.recv_timeout(IDLE_POLL)
        } else {
            requests
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match request {
            Ok(request) => {
                if request.reply.is_closed() {
                    continue;
                }
                let deadline = Instant::now() + START_TIMEOUT;
                #[cfg(test)]
                let deadline = request.refresh_deadline.unwrap_or(deadline);
                let result = refresh_connection(&mut connection, &registry, deadline)
                    .map(|()| registry.lock().unwrap().snapshot(&request.language));
                let _ = request.reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pipewire_node_names_remain_searchable_without_application_properties() {
        let mut properties = pulse::proplist::Proplist::new().unwrap();
        properties
            .set_str("node.name", "Synthetic PipeWire Player")
            .unwrap();
        assert_eq!(
            application_name(&properties).as_deref(),
            Some("Synthetic PipeWire Player")
        );
        properties.set_str("application.name", " \n").unwrap();
        assert_eq!(
            application_name(&properties).as_deref(),
            Some("Synthetic PipeWire Player")
        );
        properties
            .set_str("application.name", "Pulse Player")
            .unwrap();
        assert_eq!(
            application_name(&properties).as_deref(),
            Some("Pulse Player")
        );
        assert!(application_name(&pulse::proplist::Proplist::new().unwrap()).is_none());
    }

    #[tokio::test]
    #[ignore = "metadata only; requires MIMI_TEST_PLAYBACK_NODE on the desktop server"]
    async fn live_playback_node_is_discoverable_without_capture() {
        let expected = std::env::var("MIMI_TEST_PLAYBACK_NODE").unwrap();
        let choices = list("en".into()).await.unwrap();
        let target = choices
            .applications
            .iter()
            .find(|app| app.name.starts_with(&format!("{expected} · ")))
            .expect("existing playback node must be listed under its application name");
        assert!(resolve(&target.id).is_ok());
        eprintln!("linux live metadata discovered=true identityResolved=true capture=false");
    }

    #[test]
    fn cached_choices_do_not_initialize_discovery() {
        let initialized = SERVICE.get().is_some();
        let choices = cached("en");
        assert!(choices.supported);
        assert_eq!(SERVICE.get().is_some(), initialized);
        if !initialized {
            assert!(choices.applications.is_empty());
        }
    }

    fn metadata(index: u32) -> Metadata {
        Metadata {
            index,
            sink: 7,
            stream: None,
            icon_source: Default::default(),
        }
    }
    #[test]
    fn refresh_preserves_live_tokens_but_reuse_disconnect_and_late_callbacks_do_not() {
        let mut registry = Registry {
            connected: true,
            ..Default::default()
        };
        assert!(registry.reconcile(registry.revision(), BTreeMap::from([(1, metadata(1))])));
        let token = registry.entries[&1].0.clone();
        assert!(registry.reconcile(registry.revision(), BTreeMap::from([(1, metadata(1))])));
        assert_eq!(registry.resolve(&token).unwrap(), (1, 7));
        let before_removal = registry.revision();
        registry.event(1, Operation::Removed);
        assert!(!registry.reconcile(before_removal, BTreeMap::from([(1, metadata(1))])));
        assert!(registry.resolve(&token).is_err());
        registry.event(1, Operation::New);
        assert!(registry.reconcile(registry.revision(), BTreeMap::from([(1, metadata(1))])));
        assert!(registry.resolve(&token).is_err());
        let token = registry.entries[&1].0.clone();
        let before_disconnect = registry.revision();
        registry.invalidate();
        registry.connected = true;
        assert!(!registry.reconcile(before_disconnect, BTreeMap::from([(1, metadata(1))])));
        assert!(registry.reconcile(registry.revision(), BTreeMap::from([(1, metadata(1))])));
        assert!(registry.resolve(&token).is_err());
    }
    #[test]
    fn names_are_localized_sanitized_disambiguated_and_registry_is_bounded() {
        let mut registry = Registry {
            connected: true,
            ..Default::default()
        };
        registry.reconcile(0, (0..600).map(|i| (i, metadata(i))).collect());
        let snapshot = registry.snapshot("zh");
        assert_eq!(snapshot.applications.len(), LIMIT);
        assert!(snapshot.applications[0]
            .name
            .starts_with("未知应用 · 未命名音频流 · #"));
        assert_eq!(clean_name(Some(" \nApp\0 ")), Some("App".into()));
        assert_eq!(clean_name(Some("\n")), None);
        assert_eq!(clean_name(Some(&"a".repeat(500))).unwrap().len(), 100);
        registry.event(2, Operation::Changed);
        let token = registry.entries[&2].0.clone();
        let mut moved = metadata(2);
        moved.sink = 8;
        registry.reconcile(registry.revision(), BTreeMap::from([(2, moved)]));
        assert_eq!(registry.resolve(&token).unwrap(), (2, 8));
    }

    #[test]
    fn unchanged_capture_validation_does_not_lock_the_registry() {
        let mut registry = Registry {
            connected: true,
            ..Default::default()
        };
        registry.reconcile(0, BTreeMap::from([(1, metadata(1))]));
        let token = registry.entries[&1].0.clone();
        let (requests, _receiver) = mpsc::sync_channel(1);
        let service = Arc::new(Service {
            requests,
            revision: Arc::clone(&registry.revision),
            registry: Arc::new(Mutex::new(registry)),
        });
        let mut last_revision = None;
        assert_eq!(
            service
                .resolve_if_changed(&token, &mut last_revision)
                .unwrap(),
            Some((1, 7))
        );
        let guard = service.registry.lock().unwrap();
        let validation = Arc::clone(&service);
        let (reply, response) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            reply
                .send(validation.resolve_if_changed(&token, &mut last_revision))
                .unwrap();
        });
        let result = response.recv_timeout(Duration::from_secs(1));
        drop(guard);
        worker.join().unwrap();
        assert_eq!(result.unwrap().unwrap(), None);
    }

    #[test]
    fn capture_rechecks_event_then_completed_refresh_and_disconnect() {
        let mut registry = Registry {
            connected: true,
            ..Default::default()
        };
        registry.reconcile(0, BTreeMap::from([(1, metadata(1))]));
        let token = registry.entries[&1].0.clone();
        let (requests, _receiver) = mpsc::sync_channel(1);
        let service = Service {
            requests,
            revision: Arc::clone(&registry.revision),
            registry: Arc::new(Mutex::new(registry)),
        };
        let mut last_revision = None;
        assert_eq!(
            service
                .resolve_if_changed(&token, &mut last_revision)
                .unwrap(),
            Some((1, 7))
        );
        service
            .registry
            .lock()
            .unwrap()
            .event(1, Operation::Changed);
        assert_eq!(
            service
                .resolve_if_changed(&token, &mut last_revision)
                .unwrap(),
            Some((1, 7))
        );
        {
            let mut registry = service.registry.lock().unwrap();
            let revision = registry.revision();
            let mut moved = metadata(1);
            moved.sink = 8;
            assert!(registry.reconcile(revision, BTreeMap::from([(1, moved)])));
            assert!(!registry.reconcile(revision, BTreeMap::from([(1, metadata(1))])));
        }
        assert_eq!(
            service
                .resolve_if_changed(&token, &mut last_revision)
                .unwrap(),
            Some((1, 8))
        );
        service
            .registry
            .lock()
            .unwrap()
            .event(1, Operation::Removed);
        assert!(matches!(
            service.resolve_if_changed(&token, &mut last_revision),
            Err(Error::ApplicationUnavailable)
        ));
        service.registry.lock().unwrap().invalidate();
        assert!(matches!(
            service.resolve_if_changed(&token, &mut last_revision),
            Err(Error::ApplicationUnavailable)
        ));
    }

    #[test]
    fn only_duplicate_display_names_receive_stream_numbers() {
        let mut registry = Registry {
            connected: true,
            ..Default::default()
        };
        let mut named = metadata(3);
        named.icon_source.name = Some("Player".into());
        named.stream = Some("Music".into());
        registry.reconcile(
            0,
            BTreeMap::from([(1, metadata(1)), (2, metadata(2)), (3, named)]),
        );
        let names: Vec<_> = registry
            .snapshot("en")
            .applications
            .into_iter()
            .map(|app| app.name)
            .collect();
        assert_eq!(
            names,
            [
                "Player · Music",
                "Unknown application · Unnamed audio stream · #1",
                "Unknown application · Unnamed audio stream · #2"
            ]
        );
    }

    #[test]
    fn name_cleaning_preserves_utf8_byte_limits_and_filter_before_trim_order() {
        assert_eq!(clean_name(None), None);
        assert_eq!(clean_name(Some("\u{2003}\n\0 \t")), None);
        assert_eq!(
            clean_name(Some(" \n播\0放\t 器\u{2003}")),
            Some("播放 器".into())
        );
        assert_eq!(
            clean_name(Some(&format!("{}播", "a".repeat(99)))).unwrap(),
            "a".repeat(99)
        );
        assert_eq!(clean_name(Some(&"播".repeat(40))).unwrap(), "播".repeat(33));
        assert_eq!(
            clean_name(Some(&format!("{}abc", " ".repeat(99)))),
            Some("a".into())
        );
        assert_eq!(
            clean_name(Some(&format!("{}abc", "\0".repeat(101)))),
            Some("abc".into())
        );
    }
}
