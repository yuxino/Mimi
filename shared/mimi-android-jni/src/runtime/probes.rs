//! Native connection-probe handles, cancellation and completed diagnostics.
//! Probe tasks have a separate bounded registry from listening sessions and use
//! independent immutable speech/text networks from the native request snapshot.

use super::{
    executor, json, Arc, AtomicU64, HashMap, LiveTranslationConfiguration, Mutex, OnceLock,
    Ordering, ProxyConfig, SourceLanguage, StageNetworkInput, TargetLanguage, Value,
};
use serde::Deserialize;
use tokio::task::JoinHandle;

const MAX_PROBES: usize = 2;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextProbeInput {
    credentials: mimi_runtime::core::services::credentials::TextTranslationCredentials,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    qwen_mt_model: mimi_runtime::core::protocols::translation::qwen_mt::QwenMTModel,
}
pub(super) fn text_probe_configuration(
    value: &Value,
) -> Result<
    mimi_runtime::core::services::configuration::TextTranslationProbeConfiguration,
    &'static str,
> {
    if let Some(configuration) = value.get("sessionConfiguration") {
        let configuration: LiveTranslationConfiguration =
            serde_json::from_value(configuration.clone()).map_err(|_| "invalid_configuration")?;
        return configuration
            .xai_text_probe_configuration()
            .map_err(|_| "invalid_configuration");
    }
    let input: TextProbeInput =
        serde_json::from_value(value.clone()).map_err(|_| "invalid_configuration")?;
    Ok(
        mimi_runtime::core::services::configuration::TextTranslationProbeConfiguration {
            credentials:
                mimi_runtime::core::services::configuration::TextTranslationProbeCredentials::Independent(
                    input.credentials,
                ),
            source_language: input.source_language,
            target_language: input.target_language,
            qwen_mt_model: input.qwen_mt_model,
            network_proxy: ProxyConfig {
                mode: mimi_runtime::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            },
        },
    )
}
struct TextProbe {
    result: Mutex<
        Option<mimi_runtime::clients::transport::connection_diagnostics::ConnectionDiagnostic>,
    >,
    task: Mutex<Option<JoinHandle<()>>>,
}
fn probes() -> &'static Mutex<HashMap<u64, Arc<TextProbe>>> {
    static PROBES: OnceLock<Mutex<HashMap<u64, Arc<TextProbe>>>> = OnceLock::new();
    PROBES.get_or_init(Default::default)
}

pub(super) fn endpoint(value: &Value) -> Result<Value, &'static str> {
    let configuration = text_probe_configuration(&value["configuration"])?;
    let endpoint =
        mimi_runtime::clients::transport::connection_diagnostics::text_probe_network_endpoint(
            &configuration,
        )
        .map_err(|_| "invalid_configuration")?;
    Ok(json!({"textEndpoint":endpoint}))
}

pub(super) fn create(value: &Value) -> Result<Value, &'static str> {
    let speech = value["operation"] == "create_speech_probe";
    let speech_configuration = if speech {
        let configuration: LiveTranslationConfiguration =
            serde_json::from_value(value["configuration"].clone())
                .map_err(|_| "invalid_configuration")?;
        let configuration = configuration
            .validated()
            .map_err(|_| "invalid_configuration")?;
        if !configuration.uses_independent_asr() {
            return Err("invalid_configuration");
        }
        Some(configuration)
    } else {
        None
    };
    let configuration = if speech {
        None
    } else {
        Some(text_probe_configuration(&value["configuration"])?)
    };
    let networks: StageNetworkInput =
        serde_json::from_value(value["network"].clone()).map_err(|_| "native_network_invalid")?;
    let (speech_network, text_network) = networks.resolve()?;
    let mut registry = probes().lock().map_err(|_| "native_runtime_failed")?;
    if registry.len() >= MAX_PROBES {
        return Err("native_runtime_capacity");
    }
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let probe = Arc::new(TextProbe {
        result: Mutex::new(None),
        task: Mutex::new(None),
    });
    let worker = probe.clone();
    *probe.task.lock().unwrap() = Some(executor().spawn(async move {
        let result = if let Some(configuration) = speech_configuration {
            mimi_runtime::clients::transport::connection_diagnostics::check_independent_speech_with_network(
                &configuration,
                false,
                speech_network,
                text_network,
            )
            .await
        } else {
            mimi_runtime::clients::transport::connection_diagnostics::check_text_service_with_network(
                &configuration.expect("text probe configuration"),
                text_network,
            )
            .await
        };
        *worker.result.lock().unwrap() = Some(result);
    }));
    registry.insert(id, probe);
    Ok(json!({"handle":id}))
}

pub(super) fn poll(value: &Value) -> Result<Value, &'static str> {
    let id = value["handle"]
        .as_u64()
        .ok_or("native_runtime_invalid_request")?;
    let registry = probes().lock().map_err(|_| "native_runtime_failed")?;
    let probe = registry.get(&id).ok_or("native_runtime_stale_handle")?;
    let result = probe.result.lock().unwrap();
    Ok(json!({"finished":result.is_some(),"result":*result}))
}

pub(super) fn cancel(value: &Value) -> Result<Value, &'static str> {
    let id = value["handle"]
        .as_u64()
        .ok_or("native_runtime_invalid_request")?;
    if let Some(probe) = probes()
        .lock()
        .map_err(|_| "native_runtime_failed")?
        .remove(&id)
    {
        if let Some(task) = probe.task.lock().unwrap().take() {
            task.abort();
        }
    }
    Ok(json!({}))
}
